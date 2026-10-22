use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use matrix_sdk::Room;
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk_ui::timeline::{Timeline, TimelineItem, TimelineItemContent, TimelineReadReceiptTracking,
    TimelineItemKind, RoomExt, EventTimelineItem, VirtualTimelineItem,
    MsgLikeKind, MembershipChange, AnyOtherFullStateEventContent,
    TimelineDetails, TimelineEventItemId};
use matrix_sdk::ruma::events::FullStateEventContent;
use matrix_sdk::ruma::{OwnedRoomId, RoomId, events::room::message::MessageType};
use matrix_sdk_ui::eyeball_im::{Vector, VectorDiff};
use futures_util::{Stream, StreamExt};
use tokio::sync::{mpsc, watch};
use crate::events::{CoreEvent, MatrixEvent};
use crate::matrix::name_colors::NameColorSender;
use crate::matrix::room_activity::{ActivityFeed, ActivityWatch};
use crate::models::ChatMessageReceive;
use crate::models::MediaInfo;
use crate::models::SenderProfile;
use crate::scripting::ScriptDispatcher;
use crate::task::AbortOnDrop;
use serde::{Deserialize, Serialize};

/// Bounded cache for encrypted media source metadata (key material, IV, hashes).
/// Entries are evicted in insertion order when the cap is reached. This is safe
/// because the Matrix SDK caches decrypted media bytes separately; the source
/// metadata is only needed for the first decryption of a given mxc URL.
pub struct BoundedMediaSources {
    map: HashMap<String, MediaSource>,
    order: VecDeque<String>,
    capacity: usize,
}

const MEDIA_SOURCE_CACHE_CAP: usize = 1024;

impl BoundedMediaSources {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn insert(&mut self, key: String, source: MediaSource) {
        if self.map.contains_key(&key) {
            return;
        }
        if self.order.len() >= self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            self.map.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.map.insert(key, source);
    }

    pub fn get(&self, key: &str) -> Option<&MediaSource> {
        self.map.get(key)
    }
}

pub type MediaSourceMap = Arc<RwLock<BoundedMediaSources>>;

/// Everything kept for one subscribed room; dropping it is the teardown, so a resubscribe
/// cannot leave a duplicate stream or handler running.
struct RoomSubscription {
    timeline: Arc<Timeline>,
    _diff_task: AbortOnDrop,
    _activity: ActivityWatch,
}

pub struct TimelineManager {
    timelines: HashMap<OwnedRoomId, RoomSubscription>,
    event_tx: mpsc::Sender<CoreEvent>,
    pub media_sources: MediaSourceMap,
    dispatcher: Arc<ScriptDispatcher>,
    /// The room the user is viewing. It outlives `clear`, since the frontend sends it only on change.
    viewing: watch::Sender<Option<OwnedRoomId>>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum StateEventKind {
    RoomNameChanged { name: String },
    RoomTopicChanged { topic: String },
    RoomAvatarChanged { url: Option<String> },
    MemberJoined { user_id: String },
    MemberLeft { user_id: String },
    MemberInvited { user_id: String },
    MemberBanned { user_id: String },
    Other,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum TimelineEntryKind {
    Message(Box<ChatMessageReceive>),
    StateEvent(StateEventKind),
    DayDivider(u128),
    ReadMarker,
    Redacted,
    /// An encrypted message this device has no key for; the timeline turns it into a `Message` if the key arrives.
    Undecryptable,
    Other,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TimelineEntry {
    pub sender: Option<SenderProfile>,
    pub kind: TimelineEntryKind,
}

impl TimelineManager {
    pub fn new(event_tx: mpsc::Sender<CoreEvent>, dispatcher: Arc<ScriptDispatcher>) -> Self {
        Self {
            timelines: HashMap::new(),
            event_tx,
            media_sources: Arc::new(RwLock::new(BoundedMediaSources::new(MEDIA_SOURCE_CACHE_CAP))),
            dispatcher,
            viewing: watch::Sender::new(None),
        }
    }

    /// Each room's activity watch keeps the viewed room read.
    pub fn view(&self, room_id: Option<OwnedRoomId>) {
        self.viewing.send_replace(room_id);
    }

    /// Clear all timeline subscriptions and the media source cache.
    pub fn clear(&mut self) {
        self.timelines.clear();
        let mut sources = self.media_sources.write().expect("media source lock");
        sources.map.clear();
        sources.order.clear();
    }

    // Subscribe to a room's timeline and activity. Sends the initial batch of messages,
    // then spawns a task that loops on the diff stream forwarding changes
    // as CoreEvents. Does NOT paginate — call paginate_backwards separately.
    // `name_colors` must be the current session's.
    pub async fn subscribe_to_room(&mut self, room: &Room, name_colors: NameColorSender) {
        let room_id = room.room_id();
        // Receipts are tracked so the room's unread state and `mark_as_read` judge them alike.
        let built = room.timeline_builder()
            .track_read_marker_and_receipts(TimelineReadReceiptTracking::AllEvents)
            .build().await;
        let Ok(timeline) = built else {
            log::error!("Failed to get timeline for room: {}", room_id);
            return;
        };
        // Dropped before the replacement starts, so the two never run side by side.
        if self.timelines.remove(room_id).is_some() {
            log::info!("[timeline] Room {room_id} resubscribed, replacing its previous subscription");
        }
        let (initial_items, stream) = timeline.subscribe().await;

        let feed = self.feed(room_id);
        feed.send_initial(&initial_items).await;
        let diff_task = AbortOnDrop::new(tokio::spawn(feed.forward(stream)));
        let timeline = Arc::new(timeline);
        let activity = ActivityWatch::start(room, timeline.clone(), self.viewing.subscribe(), ActivityFeed {
            event_tx: self.event_tx.clone(),
            dispatcher: self.dispatcher.clone(),
            name_colors,
        });

        self.timelines.insert(
            room_id.to_owned(),
            RoomSubscription { timeline, _diff_task: diff_task, _activity: activity },
        );
    }

    fn feed(&self, room_id: &RoomId) -> RoomFeed {
        RoomFeed {
            room_id: room_id.to_string(),
            event_tx: self.event_tx.clone(),
            sources: self.media_sources.clone(),
        }
    }

    /// Returns cloned Arc handles for all subscribed timelines.
    /// Used to spawn background pagination without borrowing &self.
    pub fn timeline_arcs(&self) -> Vec<(OwnedRoomId, Arc<Timeline>)> {
        self.timelines.iter().map(|(k, v)| (k.clone(), v.timeline.clone())).collect()
    }

    // Request older messages for a room's timeline (triggered by user scrolling up).
    // Results arrive through the existing subscription stream as PushFront diffs.
    // Returns false if there are no more messages to load, or if the room isn't subscribed.
    pub async fn paginate_backwards(&self, room_id: &OwnedRoomId, count: u16) -> bool {
        let Some(entry) = self.timelines.get(room_id) else {
            log::error!("No timeline subscription for room: {}", room_id);
            return false;
        };

        match entry.timeline.paginate_backwards(count).await {
            Ok(hit_start) => !hit_start,
            Err(e) => {
                log::error!("Pagination error for room {}: {:?}", room_id, e);
                false
            }
        }
    }

    /// Send a message through the timeline's send queue, producing an
    /// immediate local echo in the diff stream. Returns true if the
    /// timeline was found and the send was queued.
    pub async fn send_message(
        &self,
        room_id: &str,
        content: matrix_sdk::ruma::events::AnyMessageLikeEventContent,
    ) -> bool {
        let Ok(room_id) = OwnedRoomId::try_from(room_id) else { return false };
        let Some(entry) = self.timelines.get(&room_id) else { return false };
        if let Err(e) = entry.timeline.send(content).await {
            log::error!("Failed to send message via timeline: {:?}", e);
        }
        true
    }

    /// Resolve a (room_id, event_id) pair into a timeline handle and item identifier.
    /// Returns None and logs on any parsing or lookup failure.
    fn resolve_timeline_item(&self, room_id: &str, event_id: &str, op: &str)
        -> Option<(Arc<Timeline>, TimelineEventItemId)>
    {
        let Ok(room_id) = OwnedRoomId::try_from(room_id) else {
            log::error!("[{}] Invalid room_id: {}", op, room_id);
            return None;
        };
        let Some(entry) = self.timelines.get(&room_id) else {
            log::error!("[{}] No timeline subscription for room: {}", op, room_id);
            return None;
        };
        let Ok(event_id) = matrix_sdk::ruma::OwnedEventId::try_from(event_id) else {
            log::error!("[{}] Invalid event_id: {}", op, event_id);
            return None;
        };
        Some((Arc::clone(&entry.timeline), TimelineEventItemId::EventId(event_id)))
    }

    pub async fn edit_message(&self, room_id: &str, event_id: &str, text: &str, html_body: Option<&str>) {
        let Some((timeline, item_id)) = self.resolve_timeline_item(room_id, event_id, "edit_message") else {
            return;
        };
        use matrix_sdk::ruma::events::room::message::{RoomMessageEventContent, RoomMessageEventContentWithoutRelation};
        use matrix_sdk::room::edit::EditedContent;
        let content = match html_body {
            Some(html) => RoomMessageEventContent::text_html(text, html),
            None => RoomMessageEventContent::text_plain(text),
        };
        let without_relation: RoomMessageEventContentWithoutRelation = content.into();
        if let Err(e) = timeline.edit(&item_id, EditedContent::RoomMessage(without_relation)).await {
            log::error!("Failed to edit message: {:?}", e);
        }
    }

    pub async fn redact_message(&self, room_id: &str, event_id: &str) {
        let Some((timeline, item_id)) = self.resolve_timeline_item(room_id, event_id, "redact_message") else {
            return;
        };
        if let Err(e) = timeline.redact(&item_id, None).await {
            log::error!("Failed to redact message: {:?}", e);
        }
    }

    pub async fn toggle_reaction(&self, room_id: &str, event_id: &str, key: &str) {
        let Some((timeline, item_id)) = self.resolve_timeline_item(room_id, event_id, "toggle_reaction") else {
            return;
        };
        if let Err(e) = timeline.toggle_reaction(&item_id, key).await {
            log::error!("Failed to toggle reaction: {:?}", e);
        }
    }
}

/// Where one room's timeline changes go.
struct RoomFeed {
    room_id: String,
    event_tx: mpsc::Sender<CoreEvent>,
    sources: MediaSourceMap,
}

impl RoomFeed {
    async fn send_initial(&self, items: &Vector<Arc<TimelineItem>>) {
        let entries: Vec<TimelineEntry> = items
            .iter()
            .map(|item| timeline_item_to_entry(item, &self.sources))
            .collect();

        if !entries.is_empty() {
            let _ = self.event_tx.send(
                CoreEvent::Matrix(MatrixEvent::TimelineAppend(self.room_id.clone(), entries))
            ).await;
        }
    }

    async fn forward(self, mut stream: impl Stream<Item = Vec<VectorDiff<Arc<TimelineItem>>>> + Unpin) {
        while let Some(diffs) = stream.next().await {
            for diff in diffs {
                if let Some(event) = map_diff(diff, &self.room_id, &self.sources)
                    && !self.send(event).await
                {
                    return;
                }
            }
        }
        log::warn!("[timeline] Diff stream ended for room {}", self.room_id);
    }

    async fn send(&self, event: CoreEvent) -> bool {
        if self.event_tx.send(event).await.is_err() {
            log::warn!("[timeline] Event channel closed for room {}, stopping diff task", self.room_id);
            return false;
        }
        true
    }
}

// Convert a TimelineItem into our ChatMessageReceive model.
fn timeline_item_to_entry(
    item: &TimelineItem,
    sources: &MediaSourceMap,
) -> TimelineEntry {
    match item.kind() {
        TimelineItemKind::Event(event) => {
            event_item_to_entry(event, sources)
        }
        TimelineItemKind::Virtual(virt) => {
            virtual_item_to_entry(virt)
        }
    }
}

fn extract_sender_profile(event: &EventTimelineItem) -> Option<SenderProfile> {
    match event.sender_profile() {
        TimelineDetails::Ready(profile) => {
            Some(SenderProfile {
                display_name: profile.display_name.clone(),
                avatar_url: profile.avatar_url.as_ref().map(|u| u.to_string()),
            })
        }
        _ => None,
    }
}

fn extract_media_url(source: &MediaSource, sources: &MediaSourceMap) -> String {
    match source {
        MediaSource::Plain(uri) => uri.to_string(),
        MediaSource::Encrypted(file) => {
            let url = file.url.to_string();
            sources.write().expect("media source lock").insert(url.clone(), source.clone());
            url
        }
    }
}

fn event_item_to_entry(
    event: &EventTimelineItem,
    sources: &MediaSourceMap,
) -> TimelineEntry {
    let sender = extract_sender_profile(event);

    let kind = match event.content() {
        TimelineItemContent::MsgLike(content) => {
            match &content.kind {
                MsgLikeKind::Message(message) => {
                    let (html_body, media) = match message.msgtype() {
                        MessageType::Text(text) => {
                            let html = text.formatted.as_ref().map(|f| f.body.clone());
                            (html, None)
                        }

                        MessageType::File(file) => {
                            let info = file.info.as_deref();
                            (None, Some(MediaInfo {
                                  mxc_url: extract_media_url(&file.source, sources),
                                  mimetype: info.and_then(|i| i.mimetype.clone()).unwrap_or_default(),
                                  size: info.and_then(|i| i.size).unwrap_or_default().into(),
                                  width: 0,
                                  height: 0,
                                  duration: 0
                              }))
                        }

                        MessageType::Image(image) => {
                            let info = image.info.as_deref();
                            (None, Some(MediaInfo {
                                  mxc_url: extract_media_url(&image.source, sources),
                                  mimetype: info.and_then(|i| i.mimetype.clone()).unwrap_or_default(),
                                  size: info.and_then(|i| i.size).unwrap_or_default().into(),
                                  width: info.and_then(|i| i.width).unwrap_or_default().into(),
                                  height: info.and_then(|i| i.height).unwrap_or_default().into(),
                                  duration: 0
                              }))
                        }

                        MessageType::Video(video) => {
                            let info = video.info.as_deref();
                            (None, Some(MediaInfo {
                                  mxc_url: extract_media_url(&video.source, sources),
                                  mimetype: info.and_then(|i| i.mimetype.clone()).unwrap_or_default(),
                                  size: info.and_then(|i| i.size).unwrap_or_default().into(),
                                  width: info.and_then(|i| i.width).unwrap_or_default().into(),
                                  height: info.and_then(|i| i.height).unwrap_or_default().into(),
                                  duration: info.and_then(|i| i.duration).unwrap_or_default().as_millis(),
                              }))
                        }

                        MessageType::Audio(audio) => {
                            let info = audio.info.as_deref();
                            (None, Some(MediaInfo {
                                  mxc_url: extract_media_url(&audio.source, sources),
                                  mimetype: info.and_then(|i| i.mimetype.clone()).unwrap_or_default(),
                                  size: info.and_then(|i| i.size).unwrap_or_default().into(),
                                  width: 0,
                                  height: 0,
                                  duration: info.and_then(|i| i.duration).unwrap_or_default().as_millis(),
                              }))
                        }
                        _ => (None, None),
                    };

                    let reactions = content.reactions.iter().map(|(key, senders)| {
                        (key.clone(), senders.keys().map(|uid| uid.to_string()).collect())
                    }).collect();

                    let ts: u64 = event.timestamp().0.into();

                    TimelineEntryKind::Message(Box::new(ChatMessageReceive {
                        id: event.event_id().map(|id| id.to_string()).unwrap_or_default(),
                        sender: event.sender().to_string(),
                        body: message.body().to_string(),
                        html_body,
                        media,
                        timestamp: ts as u128,
                        edited: message.is_edited(),
                        reactions,
                    }))
                }
                MsgLikeKind::Redacted => TimelineEntryKind::Redacted,
                MsgLikeKind::UnableToDecrypt(_) => TimelineEntryKind::Undecryptable,
                other => {
                    log::trace!("[timeline] Unhandled MsgLikeKind: {:?}", other);
                    TimelineEntryKind::Other
                }
            }
        }

        TimelineItemContent::OtherState(state) => {
            let state_kind = match state.content() {
                AnyOtherFullStateEventContent::RoomName(full) => {
                    if let FullStateEventContent::Original { content, .. } = full {
                        StateEventKind::RoomNameChanged { name: content.name.clone() }
                    } else {
                        StateEventKind::Other
                    }
                }
                AnyOtherFullStateEventContent::RoomTopic(full) => {
                    if let FullStateEventContent::Original { content, .. } = full {
                        StateEventKind::RoomTopicChanged { topic: content.topic.clone() }
                    } else {
                        StateEventKind::Other
                    }
                }
                AnyOtherFullStateEventContent::RoomAvatar(full) => {
                    if let FullStateEventContent::Original { content, .. } = full {
                        StateEventKind::RoomAvatarChanged {
                            url: content.url.as_ref().map(|u| u.to_string()),
                        }
                    } else {
                        StateEventKind::Other
                    }
                }
                _ => StateEventKind::Other,
            };
            TimelineEntryKind::StateEvent(state_kind)
        }

        TimelineItemContent::MembershipChange(change) => {
            let user_id = change.user_id().to_string();
            let state_kind = match change.change() {
                Some(MembershipChange::Joined) => StateEventKind::MemberJoined { user_id },
                Some(MembershipChange::Left) => StateEventKind::MemberLeft { user_id },
                Some(MembershipChange::Invited) => StateEventKind::MemberInvited { user_id },
                Some(MembershipChange::Banned) => StateEventKind::MemberBanned { user_id },
                _ => StateEventKind::Other,
            };
            TimelineEntryKind::StateEvent(state_kind)
        }

        _ => TimelineEntryKind::Other,
    };

    TimelineEntry { sender, kind }
}

fn virtual_item_to_entry(virt: &VirtualTimelineItem) -> TimelineEntry {
    let kind = match virt {
        VirtualTimelineItem::DateDivider(ts) => {
            let millis: u64 = ts.0.into();
            TimelineEntryKind::DayDivider(millis as u128)
        }
        VirtualTimelineItem::ReadMarker => TimelineEntryKind::ReadMarker,
        _ => TimelineEntryKind::Other,
    };
    TimelineEntry { sender: None, kind }
}

fn map_diff(
    diff: VectorDiff<Arc<TimelineItem>>,
    rid: &str,
    sources: &MediaSourceMap,
) -> Option<CoreEvent> {
    let matrix_event = match diff {
        VectorDiff::PushBack { value } => {
            Some(MatrixEvent::TimelinePushBack(rid.to_owned(), timeline_item_to_entry(&value, sources)))
        }
        VectorDiff::PushFront { value } => {
            Some(MatrixEvent::TimelinePushFront(rid.to_owned(), timeline_item_to_entry(&value, sources)))
        }
        VectorDiff::Append { values } => {
            Some(MatrixEvent::TimelineAppend(rid.to_owned(),
                values.iter().map(|item| timeline_item_to_entry(item, sources)).collect()))
        }
        VectorDiff::Set { index, value } => {
            Some(MatrixEvent::TimelineSet(rid.to_owned(), index, timeline_item_to_entry(&value, sources)))
        }
        VectorDiff::Insert { index, value } => {
            Some(MatrixEvent::TimelineInsert(rid.to_owned(), index, timeline_item_to_entry(&value, sources)))
        }
        VectorDiff::Remove { index } => {
            Some(MatrixEvent::TimelineRemove(rid.to_owned(), index))
        }
        VectorDiff::Clear => {
            log::debug!("[timeline] Room {rid} timeline cleared");
            Some(MatrixEvent::TimelineCleared(rid.to_owned()))
        }
        VectorDiff::Reset { values } => {
            log::debug!("[timeline] Room {rid} timeline reset to {} items", values.len());
            Some(MatrixEvent::TimelineReset(rid.to_owned(),
                values.iter().map(|item| timeline_item_to_entry(item, sources)).collect()))
        }
        _ => {
            log::warn!("Unhandled VectorDiff variant");
            None
        }
    };
    matrix_event.map(CoreEvent::Matrix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::OwnedMxcUri;

    fn plain_source(uri: &str) -> MediaSource {
        MediaSource::Plain(OwnedMxcUri::from(uri.to_owned()))
    }

    #[test]
    fn insert_and_get() {
        let mut cache = BoundedMediaSources::new(4);
        cache.insert("mxc://a".into(), plain_source("mxc://a"));
        assert!(cache.get("mxc://a").is_some());
    }

    #[test]
    fn duplicate_key_is_noop() {
        let mut cache = BoundedMediaSources::new(4);
        cache.insert("key".into(), plain_source("mxc://original"));
        cache.insert("key".into(), plain_source("mxc://replacement"));
        assert_eq!(cache.order.len(), 1);
        assert_eq!(cache.map.len(), 1);
    }

    #[test]
    fn evicts_oldest_at_capacity() {
        let mut cache = BoundedMediaSources::new(2);
        cache.insert("a".into(), plain_source("mxc://a"));
        cache.insert("b".into(), plain_source("mxc://b"));
        cache.insert("c".into(), plain_source("mxc://c"));

        assert!(cache.get("a").is_none(), "oldest entry should be evicted");
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
        assert_eq!(cache.map.len(), 2);
        assert_eq!(cache.order.len(), 2);
    }

    #[test]
    fn get_returns_none_for_missing() {
        let cache = BoundedMediaSources::new(4);
        assert!(cache.get("nonexistent").is_none());
    }

    #[test]
    fn clear_empties_media_sources_and_timelines() {
        // TimelineManager::clear() is the backend half of the ServerReset
        // path. Dropping each entry aborts the diff-stream task it owns, and
        // clearing media sources prevents stale encrypted-media metadata from
        // leaking across sessions.
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let mut mgr = TimelineManager::new(tx, Arc::new(ScriptDispatcher::empty()));

        // Populate the media source cache via the shared lock (same path
        // the real code takes through extract_media_url).
        {
            let mut sources = mgr.media_sources.write().unwrap();
            sources.insert("mxc://a".into(), plain_source("mxc://a"));
            sources.insert("mxc://b".into(), plain_source("mxc://b"));
        }

        mgr.clear();

        let sources = mgr.media_sources.read().unwrap();
        assert!(sources.get("mxc://a").is_none(), "media sources should be cleared");
        assert!(sources.get("mxc://b").is_none(), "media sources should be cleared");
        assert_eq!(sources.map.len(), 0);
        assert_eq!(sources.order.len(), 0);

        assert!(mgr.timeline_arcs().is_empty(), "timeline handles should be cleared");
    }

    #[test]
    fn duplicate_insert_does_not_refresh_eviction_order() {
        // Eviction is by insertion order, and a duplicate insert is a no-op, so
        // re-inserting an existing key must NOT move it to the back of the queue.
        let mut cache = BoundedMediaSources::new(3);
        cache.insert("a".into(), plain_source("mxc://a"));
        cache.insert("b".into(), plain_source("mxc://b"));
        cache.insert("c".into(), plain_source("mxc://c"));
        cache.insert("a".into(), plain_source("mxc://a-again")); // no-op
        cache.insert("d".into(), plain_source("mxc://d"));       // evicts oldest

        assert!(cache.get("a").is_none(), "a was oldest and must be evicted, not refreshed");
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
        assert!(cache.get("d").is_some());
    }
}

// Tests for the diff-to-event mapper. The SDK-driven paths (event_item_to_entry,
// subscribe/spawn) are exercised by the Docker integration suite instead.
#[cfg(test)]
mod diff_dispatch_tests {
    use super::*;

    fn sources() -> MediaSourceMap {
        Arc::new(RwLock::new(BoundedMediaSources::new(4)))
    }

    #[test]
    fn map_diff_remove_carries_index() {
        match map_diff(VectorDiff::Remove { index: 3 }, "!r:x", &sources()) {
            Some(CoreEvent::Matrix(MatrixEvent::TimelineRemove(rid, idx))) => {
                assert_eq!(rid, "!r:x");
                assert_eq!(idx, 3);
            }
            other => panic!("expected TimelineRemove, got {other:?}"),
        }
    }

    #[test]
    fn map_diff_clear_maps_to_cleared() {
        match map_diff(VectorDiff::Clear, "!r:x", &sources()) {
            Some(CoreEvent::Matrix(MatrixEvent::TimelineCleared(rid))) => assert_eq!(rid, "!r:x"),
            other => panic!("expected TimelineCleared, got {other:?}"),
        }
    }

    #[test]
    fn map_diff_unhandled_variants_are_dropped() {
        // PopFront/PopBack/Truncate have no CoreEvent equivalent and must be
        // ignored rather than mis-mapped.
        assert!(map_diff(VectorDiff::PopBack, "!r:x", &sources()).is_none());
        assert!(map_diff(VectorDiff::PopFront, "!r:x", &sources()).is_none());
        assert!(map_diff(VectorDiff::Truncate { length: 0 }, "!r:x", &sources()).is_none());
    }
}
