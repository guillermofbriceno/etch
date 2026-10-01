//! The color each user's name is shown in, read from the `etch.name_color` profile field.
//!
//! `NameColorCache` decides what to fetch and what to announce, and never sees a client or
//! a clock; `NameColorResolver` is the task that acts on it.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use matrix_sdk::Client;
use matrix_sdk::ruma::UserId;
use matrix_sdk::ruma::api::{MatrixVersion, SupportedVersions};
use matrix_sdk::ruma::api::client::discovery::get_capabilities::v3::Capabilities;
use matrix_sdk::ruma::api::client::error::ErrorKind;
use matrix_sdk::ruma::api::client::profile::{ProfileFieldName, ProfileFieldValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::events::{CoreEvent, MatrixEvent};
use crate::task::AbortOnDrop;

const FIELD: &str = "etch.name_color";
const EXTENDED_PROFILES_FEATURE: &str = "uk.tcpip.msc4133";

const MAX_IN_FLIGHT: usize = 4;
const ACTIVITY_REFRESH_AFTER: Duration = Duration::from_secs(60);
const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const INPUT_QUEUE: usize = 256;

/// Checked for form only: whether a color is readable is for the frontend to decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonMap<String, JsonValue>", into = "JsonMap<String, JsonValue>")]
pub struct NameColor {
    rgb: [u8; 3],
}

/// Parsed from a map rather than derived, because a derived struct also accepts `[color]`.
impl TryFrom<JsonMap<String, JsonValue>> for NameColor {
    type Error = &'static str;

    fn try_from(object: JsonMap<String, JsonValue>) -> Result<Self, Self::Error> {
        object.get("color")
            .and_then(JsonValue::as_str)
            .and_then(Self::parse)
            .ok_or("a name color is {\"color\": \"#rrggbb\"}")
    }
}

impl From<NameColor> for JsonMap<String, JsonValue> {
    fn from(color: NameColor) -> Self {
        let [r, g, b] = color.rgb;
        Self::from_iter([("color".to_owned(), format!("#{r:02x}{g:02x}{b:02x}").into())])
    }
}

impl NameColor {
    fn parse(hex: &str) -> Option<Self> {
        let digits = hex.strip_prefix('#')?;
        // `from_str_radix` alone would also accept a leading `+`.
        if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let [_, r, g, b] = u32::from_str_radix(digits, 16).ok()?.to_be_bytes();
        Some(Self { rgb: [r, g, b] })
    }

    /// Anything that is not a valid `etch.name_color` value means the user has no color.
    fn from_profile(value: &ProfileFieldValue) -> Option<Self> {
        if value.field_name().as_str() != FIELD {
            return None;
        }
        Self::deserialize(&*value.value()).ok()
    }

    pub(crate) fn to_profile(self) -> ProfileFieldValue {
        let value = serde_json::to_value(self).expect("a name color always serializes");
        ProfileFieldValue::new(FIELD, value).expect("a custom profile field accepts any JSON value")
    }
}

pub(crate) fn field_name() -> ProfileFieldName {
    ProfileFieldName::from(FIELD)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserNameColor {
    pub user_id: String,
    pub color: Option<NameColor>,
}

/// Whether the server can hold custom profile fields at all.
fn speaks_v1_16(supported: &SupportedVersions) -> bool {
    supported.versions.iter().any(|version| version.is_superset_of(MatrixVersion::V1_16))
}

fn reads_supported(supported: &SupportedVersions) -> bool {
    speaks_v1_16(supported)
        || supported.features.iter().any(|feature| feature.as_str() == EXTENDED_PROFILES_FEATURE)
}

fn settable(supported: &SupportedVersions, capabilities: &Capabilities) -> bool {
    match &capabilities.profile_fields {
        Some(fields) => reads_supported(supported) && fields.can_set_field(&field_name()),
        // The spec says a 1.16 server that omits the capability allows every field.
        None => speaks_v1_16(supported),
    }
}

/// Whether the local user can set their own name color on this server.
pub(crate) async fn settable_on(client: &Client) -> bool {
    let supported = match client.supported_versions().await {
        Ok(supported) => supported,
        Err(e) => {
            log::warn!("Could not read the homeserver's supported versions: {e}");
            return false;
        }
    };
    let capabilities = match client.get_capabilities().await {
        Ok(capabilities) => capabilities,
        Err(e) => {
            log::warn!("Could not read the homeserver's capabilities: {e}");
            return false;
        }
    };
    settable(&supported, &capabilities)
}

/// Whether a failed lookup is the server's answer that the user has no color.
fn means_no_color(kind: Option<&ErrorKind>) -> bool {
    matches!(kind, Some(ErrorKind::NotFound | ErrorKind::Forbidden { .. }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fetched {
    Value(Option<NameColor>),
    /// Says nothing about the user's color, so the last known value stands.
    Failed,
}

#[derive(Debug, Default)]
enum Fetch {
    #[default]
    Idle,
    Queued,
    InFlight { since: Instant },
}

#[derive(Debug, Default)]
struct Entry {
    color: Option<NameColor>,
    /// When the server last gave an answer; `None` means the user is not known yet.
    fetched_at: Option<Instant>,
    fetch: Fetch,
    waiting: bool,
}

impl Entry {
    fn age(&self, now: Instant) -> Option<Duration> {
        self.fetched_at.map(|at| now.duration_since(at))
    }
}

/// Answers every lookup, including "no color", so the frontend and this cache can each be
/// reset on their own without ever disagreeing.
#[derive(Debug, Default)]
pub(crate) struct NameColorCache {
    users: HashMap<String, Entry>,
    queue: VecDeque<String>,
    in_flight: usize,
}

fn schedule(queue: &mut VecDeque<String>, user: &str, entry: &mut Entry) {
    if matches!(entry.fetch, Fetch::Idle) {
        entry.fetch = Fetch::Queued;
        queue.push_back(user.to_owned());
    }
}

impl NameColorCache {
    /// Known users are answered now; the rest are answered when their fetch lands.
    pub(crate) fn lookup(&mut self, users: Vec<String>) -> Vec<UserNameColor> {
        let mut answers = Vec::new();
        for user in users {
            let entry = self.users.entry(user.clone()).or_default();
            if entry.fetched_at.is_some() {
                answers.push(UserNameColor { user_id: user, color: entry.color });
            } else {
                entry.waiting = true;
                schedule(&mut self.queue, &user, entry);
            }
        }
        answers
    }

    pub(crate) fn saw_activity(&mut self, user: String, now: Instant) {
        let entry = self.users.entry(user.clone()).or_default();
        if entry.age(now).is_none_or(|age| age >= ACTIVITY_REFRESH_AFTER) {
            schedule(&mut self.queue, &user, entry);
        }
    }

    pub(crate) fn refresh_stale(&mut self, now: Instant) {
        for (user, entry) in &mut self.users {
            if entry.age(now).is_none_or(|age| age >= REFRESH_INTERVAL) {
                schedule(&mut self.queue, user, entry);
            }
        }
    }

    /// A value known without a fetch, such as the one the local user just set.
    pub(crate) fn learned(&mut self, user: String, color: Option<NameColor>, now: Instant) -> UserNameColor {
        let entry = self.users.entry(user.clone()).or_default();
        entry.color = color;
        entry.fetched_at = Some(now);
        entry.waiting = false;
        UserNameColor { user_id: user, color }
    }

    /// The users whose fetch should start now; the caller must report each with `fetched`.
    pub(crate) fn start_fetches(&mut self, now: Instant) -> Vec<String> {
        let mut started = Vec::new();
        while self.in_flight < MAX_IN_FLIGHT
            && let Some(user) = self.queue.pop_front()
        {
            let Some(entry) = self.users.get_mut(&user) else { continue };
            entry.fetch = Fetch::InFlight { since: now };
            self.in_flight += 1;
            started.push(user);
        }
        started
    }

    pub(crate) fn fetched(&mut self, user: &str, outcome: Fetched, now: Instant) -> Option<UserNameColor> {
        let entry = self.users.get_mut(user)?;
        let Fetch::InFlight { since } = entry.fetch else { return None };
        entry.fetch = Fetch::Idle;
        self.in_flight -= 1;

        // A fetch sent before a value was learned may carry the value that one replaced.
        let superseded = entry.fetched_at.is_some_and(|at| at >= since);
        let changed = match outcome {
            Fetched::Value(color) if !superseded => {
                entry.fetched_at = Some(now);
                std::mem::replace(&mut entry.color, color) != color
            }
            _ => false,
        };

        let waiting = std::mem::take(&mut entry.waiting);
        (changed || waiting).then(|| UserNameColor { user_id: user.to_owned(), color: entry.color })
    }
}

pub(crate) enum Input {
    Lookup(Vec<String>),
    Activity(String),
    Learned { user_id: String, color: Option<NameColor> },
}

/// Never awaits, like a media fetch handed to the Matrix actor: a request that does not fit
/// is logged and dropped.
#[derive(Clone)]
pub(crate) struct NameColorSender(mpsc::Sender<Input>);

impl NameColorSender {
    pub(crate) fn lookup(&self, user_ids: Vec<String>) {
        self.offer(Input::Lookup(user_ids));
    }

    pub(crate) fn activity(&self, user_id: &str) {
        self.offer(Input::Activity(user_id.to_owned()));
    }

    pub(crate) fn learned(&self, user_id: String, color: Option<NameColor>) {
        self.offer(Input::Learned { user_id, color });
    }

    fn offer(&self, input: Input) {
        match self.0.try_send(input) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                log::warn!("Dropping a name color request: the resolver's queue is full");
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                log::debug!("Dropping a name color request: its resolver has stopped");
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn test_channel() -> (Self, mpsc::Receiver<Input>) {
        let (tx, rx) = mpsc::channel(INPUT_QUEUE);
        (Self(tx), rx)
    }
}

/// One per Matrix client, owned by the session that owns the client, so it survives
/// reconnects and cannot keep a replaced client alive.
pub(crate) struct NameColorResolver {
    sender: NameColorSender,
    _task: AbortOnDrop,
}

impl NameColorResolver {
    pub(crate) fn spawn(client: Client, event_tx: mpsc::Sender<CoreEvent>) -> Self {
        let (tx, rx) = mpsc::channel(INPUT_QUEUE);
        let task = AbortOnDrop::new(tokio::spawn(resolve(client, rx, event_tx)));
        Self { sender: NameColorSender(tx), _task: task }
    }

    pub(crate) fn sender(&self) -> &NameColorSender {
        &self.sender
    }
}

async fn resolve(client: Client, mut inputs: mpsc::Receiver<Input>, event_tx: mpsc::Sender<CoreEvent>) {
    let mut cache = NameColorCache::default();
    let mut fetches = JoinSet::new();
    let mut fetching: HashMap<tokio::task::Id, String> = HashMap::new();
    let mut refresh = tokio::time::interval_at(Instant::now() + REFRESH_INTERVAL, REFRESH_INTERVAL);

    loop {
        let answers = tokio::select! {
            input = inputs.recv() => match input {
                Some(Input::Lookup(users)) => cache.lookup(users),
                Some(Input::Activity(user)) => {
                    cache.saw_activity(user, Instant::now());
                    Vec::new()
                }
                Some(Input::Learned { user_id, color }) => vec![cache.learned(user_id, color, Instant::now())],
                None => break,
            },

            Some(joined) = fetches.join_next_with_id() => {
                let (id, outcome) = joined.unwrap_or_else(|e| {
                    log::error!("A name color fetch did not finish: {e}");
                    (e.id(), Fetched::Failed)
                });
                let Some(user) = fetching.remove(&id) else { continue };
                cache.fetched(&user, outcome, Instant::now()).into_iter().collect()
            },

            _ = refresh.tick() => {
                cache.refresh_stale(Instant::now());
                Vec::new()
            },
        };

        for user in cache.start_fetches(Instant::now()) {
            let handle = fetches.spawn(fetch(client.clone(), user.clone()));
            fetching.insert(handle.id(), user);
        }

        if !answers.is_empty()
            && event_tx.send(CoreEvent::Matrix(MatrixEvent::NameColors(answers))).await.is_err()
        {
            break;
        }
    }
}

pub(crate) async fn fetch(client: Client, user: String) -> Fetched {
    let Ok(user_id) = UserId::parse(&user) else {
        return Fetched::Value(None);
    };

    match client.supported_versions().await {
        Ok(supported) if !reads_supported(&supported) => return Fetched::Value(None),
        Ok(_) => {}
        Err(e) => {
            log::warn!("Could not read the homeserver's supported versions: {e}");
            return Fetched::Failed;
        }
    }

    match client.account().fetch_profile_field_of(user_id, field_name()).await {
        Ok(value) => Fetched::Value(value.as_ref().and_then(NameColor::from_profile)),
        Err(e) if means_no_color(e.client_api_error_kind()) => Fetched::Value(None),
        Err(e) => {
            log::warn!("Could not fetch the name color of {user}: {e}");
            Fetched::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ALICE: &str = "@alice:example.org";
    const BOB: &str = "@bob:example.org";
    const CAROL: &str = "@carol:example.org";

    fn color(hex: &str) -> Option<NameColor> {
        Some(NameColor::parse(hex).expect("a valid color"))
    }

    fn answer(user: &str, color: Option<NameColor>) -> UserNameColor {
        UserNameColor { user_id: user.into(), color }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn users(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn known(cache: &mut NameColorCache, user: &str, color: Option<NameColor>, at: Instant) {
        cache.lookup(users(&[user]));
        assert_eq!(cache.start_fetches(at), users(&[user]));
        cache.fetched(user, Fetched::Value(color), at);
    }

    #[test]
    fn every_lookup_is_answered() {
        let mut cache = NameColorCache::default();
        let t0 = Instant::now();

        cache.saw_activity(CAROL.into(), t0);
        assert_eq!(cache.start_fetches(t0), users(&[CAROL]));
        assert!(cache.lookup(users(&[ALICE, BOB, CAROL])).is_empty(), "nothing is known yet");
        assert_eq!(cache.start_fetches(t0), users(&[ALICE, BOB]), "carol's fetch in flight must be reused");

        assert_eq!(cache.fetched(ALICE, Fetched::Value(color("#51caa4")), t0), Some(answer(ALICE, color("#51caa4"))));
        assert_eq!(cache.fetched(BOB, Fetched::Failed, t0), Some(answer(BOB, None)), "a failed fetch still answers");
        assert_eq!(
            cache.fetched(CAROL, Fetched::Value(None), t0),
            Some(answer(CAROL, None)),
            "no color is an answer too, even from a fetch the lookup did not start",
        );

        assert_eq!(cache.lookup(users(&[ALICE, CAROL])), vec![answer(ALICE, color("#51caa4")), answer(CAROL, None)]);
        assert!(cache.start_fetches(t0).is_empty(), "a cached answer needs no fetch");
    }

    #[test]
    fn activity_refetches_a_user_unless_fetched_within_the_last_minute() {
        let mut cache = NameColorCache::default();
        let t0 = Instant::now();
        known(&mut cache, ALICE, color("#e99e5b"), t0);

        cache.saw_activity(ALICE.into(), t0 + secs(59));
        assert!(cache.start_fetches(t0 + secs(59)).is_empty(), "alice was fetched under a minute ago");

        cache.saw_activity(ALICE.into(), t0 + secs(60));
        assert_eq!(cache.start_fetches(t0 + secs(60)), users(&[ALICE]));
    }

    #[test]
    fn a_fetch_started_before_a_value_was_learned_does_not_overwrite_it() {
        let mut cache = NameColorCache::default();
        let t0 = Instant::now();
        known(&mut cache, ALICE, color("#c7b14d"), t0);

        let t1 = t0 + ACTIVITY_REFRESH_AFTER;
        cache.saw_activity(ALICE.into(), t1);
        cache.start_fetches(t1);
        cache.learned(ALICE.into(), color("#2bc7d6"), t1 + secs(1));

        assert_eq!(cache.fetched(ALICE, Fetched::Value(color("#c7b14d")), t1 + secs(2)), None);
        assert_eq!(cache.lookup(users(&[ALICE])), vec![answer(ALICE, color("#2bc7d6"))]);

        let t2 = t1 + secs(1) + ACTIVITY_REFRESH_AFTER;
        cache.saw_activity(ALICE.into(), t2);
        assert_eq!(cache.start_fetches(t2), users(&[ALICE]));
        assert_eq!(
            cache.fetched(ALICE, Fetched::Value(color("#eb90bd")), t2),
            Some(answer(ALICE, color("#eb90bd"))),
            "a fetch started after the value was learned applies, and the change is announced",
        );
    }

    #[test]
    fn a_failed_fetch_keeps_the_last_known_value() {
        let mut cache = NameColorCache::default();
        let t0 = Instant::now();
        known(&mut cache, ALICE, color("#e99e5b"), t0);

        let t1 = t0 + ACTIVITY_REFRESH_AFTER;
        cache.saw_activity(ALICE.into(), t1);
        cache.start_fetches(t1);
        assert_eq!(cache.fetched(ALICE, Fetched::Failed, t1), None, "nothing changed and nobody asked");
        assert_eq!(cache.lookup(users(&[ALICE])), vec![answer(ALICE, color("#e99e5b"))]);
    }

    #[test]
    fn no_more_than_four_fetches_run_at_once() {
        let mut cache = NameColorCache::default();
        let t0 = Instant::now();
        let everyone: Vec<String> = (0..6).map(|n| format!("@user{n}:example.org")).collect();

        cache.lookup(everyone.clone());
        assert_eq!(cache.start_fetches(t0), everyone[..4].to_vec());
        assert!(cache.start_fetches(t0).is_empty(), "the rest must wait for a slot");

        cache.fetched(&everyone[1], Fetched::Failed, t0);
        assert_eq!(cache.start_fetches(t0), everyone[4..5].to_vec());
        cache.fetched(&everyone[0], Fetched::Value(None), t0);
        assert_eq!(cache.start_fetches(t0), everyone[5..].to_vec());
    }

    fn from_profile(value: serde_json::Value) -> Option<NameColor> {
        NameColor::from_profile(&ProfileFieldValue::new(FIELD, value).unwrap())
    }

    #[test]
    fn a_color_is_read_in_either_case_and_written_in_lowercase() {
        let read = from_profile(json!({ "color": "#51CAA4", "hue": 120 }));
        assert_eq!(read, color("#51caa4"), "unknown keys are ignored");
        assert_eq!(*read.unwrap().to_profile().value(), json!({ "color": "#51caa4" }));
    }

    #[test]
    fn anything_but_a_hash_and_six_hex_digits_is_no_color() {
        for malformed in [
            json!({ "color": "51caa4" }),
            json!({ "color": "#5ca" }),
            json!({ "color": "#51caag" }),
            json!({ "color": "#+1caa4" }),
            json!({ "color": 0x51caa4 }),
            json!({ "hue": 120 }),
            json!(["#51caa4"]),
        ] {
            assert_eq!(from_profile(malformed.clone()), None, "{malformed} must mean no color");
        }
    }

    #[test]
    fn the_color_is_settable_only_where_it_can_be_read_and_the_capability_allows_it() {
        let enabled: Capabilities = serde_json::from_value(json!({ "m.profile_fields": { "enabled": true } })).unwrap();
        let versions = |versions: &[MatrixVersion], features: &[&str]| SupportedVersions {
            versions: versions.iter().copied().collect(),
            features: features.iter().map(|&feature| feature.into()).collect(),
        };

        assert!(settable(&versions(&[MatrixVersion::V1_16], &[]), &enabled));
        assert!(settable(&versions(&[MatrixVersion::V1_17], &[]), &enabled), "later versions include it");
        assert!(settable(&versions(&[MatrixVersion::V1_1], &[EXTENDED_PROFILES_FEATURE]), &enabled));
        assert!(!settable(&versions(&[MatrixVersion::V1_15], &[]), &enabled), "a color that cannot be read");

        let disallowed: Capabilities = serde_json::from_value(
            json!({ "m.profile_fields": { "enabled": true, "disallowed": [FIELD] } }),
        ).unwrap();
        assert!(!settable(&versions(&[MatrixVersion::V1_16], &[]), &disallowed));

        let omitted: Capabilities = serde_json::from_value(json!({})).unwrap();
        assert!(settable(&versions(&[MatrixVersion::V1_16], &[]), &omitted), "omitted means unrestricted from 1.16");
        assert!(!settable(&versions(&[MatrixVersion::V1_1], &[EXTENDED_PROFILES_FEATURE]), &omitted));
    }
}
