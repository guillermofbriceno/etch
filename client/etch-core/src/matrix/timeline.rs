use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use matrix_sdk::Room;
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk_ui::timeline::{Timeline, TimelineItem, TimelineItemContent,
    TimelineItemKind, RoomExt, EventTimelineItem, VirtualTimelineItem,
    MsgLikeKind, MembershipChange, AnyOtherFullStateEventContent,
    TimelineDetails, TimelineEventItemId};
use matrix_sdk::ruma::events::FullStateEventContent;
use matrix_sdk::ruma::{OwnedRoomId, events::room::message::MessageType};
use matrix_sdk_ui::eyeball_im::VectorDiff;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use crate::events::{CoreEvent, MatrixEvent};
use crate::models::ChatMessageReceive;
use crate::models::MediaInfo;
use crate::models::SenderProfile;
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
        if self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, source);
    }

    pub fn get(&self, key: &str) -> Option<&MediaSource> {
        self.map.get(key)
    }
}

pub type MediaSourceMap = Arc<RwLock<BoundedMediaSources>>;

pub struct TimelineManager {
    timelines: HashMap<OwnedRoomId, Arc<Timeline>>,
    event_tx: mpsc::Sender<CoreEvent>,
    pub media_sources: MediaSourceMap,
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
    Message(ChatMessageReceive),
    StateEvent(StateEventKind),
    DayDivider(u128),
    ReadMarker,
    Redacted,
    Other,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TimelineEntry {
    pub sender: Option<SenderProfile>,
    pub kind: TimelineEntryKind,
}

impl TimelineManager {
    pub fn new(event_tx: mpsc::Sender<CoreEvent>) -> Self {
        Self {
            timelines: HashMap::new(),
            event_tx,
            media_sources: Arc::new(RwLock::new(BoundedMediaSources::new(MEDIA_SOURCE_CACHE_CAP))),
        }
    }

    // Subscribe to a room's timeline. Sends the initial batch of messages,
    // then spawns a task that loops on the diff stream forwarding changes
    // as CoreEvents. Does NOT paginate — call paginate_backwards separately.
    pub async fn subscribe_to_room(&mut self, room: &Room) {
        let room_id = room.room_id();

        let Ok(timeline) = room.timeline().await else {
            log::error!("Failed to get timeline for room: {}", room_id);
            return;
        };
        let (initial_items, mut stream) = timeline.subscribe().await;

        // Send initial items as a batch
        let room_id_str = room_id.to_string();
        let messages: Vec<TimelineEntry> = initial_items
            .iter()
            .map(|item| timeline_item_to_entry(item, &self.media_sources))
            .collect();

        if !messages.is_empty() {
            let _ = self.event_tx.send(
                CoreEvent::Matrix(MatrixEvent::TimelineAppend(room_id_str.clone(), messages))
            ).await;
        }

        // Store timeline handle for pagination and reactions
        let timeline = Arc::new(timeline);
        self.timelines.insert(room_id.to_owned(), timeline);

        // Spawn a task to process the diff stream
        let event_tx = self.event_tx.clone();
        let rid = room_id_str.clone();
        let sources = self.media_sources.clone();

        tokio::spawn(async move {
            while let Some(diffs) = stream.next().await {
                for diff in diffs {
                    if let Some(entry) = map_diff(diff, &rid, &sources) {
                        let _ = event_tx.send(entry).await;
                    }
                }
            }
        });
    }

    /// Returns cloned Arc handles for all subscribed timelines.
    /// Used to spawn background pagination without borrowing &self.
    pub fn timeline_arcs(&self) -> Vec<(OwnedRoomId, Arc<Timeline>)> {
        self.timelines.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    // Request older messages for a room's timeline (triggered by user scrolling up).
    // Results arrive through the existing subscription stream as PushFront diffs.
    // Returns false if there are no more messages to load, or if the room isn't subscribed.
    pub async fn paginate_backwards(&self, room_id: &OwnedRoomId, count: u16) -> bool {
        let Some(timeline) = self.timelines.get(room_id) else {
            log::error!("No timeline subscription for room: {}", room_id);
            return false;
        };

        match timeline.paginate_backwards(count).await {
            Ok(hit_start) => !hit_start,
            Err(e) => {
                log::error!("Pagination error for room {}: {:?}", room_id, e);
                false
            }
        }
    }

    pub async fn toggle_reaction(&self, room_id: &str, event_id: &str, key: &str) {
        let Ok(room_id) = OwnedRoomId::try_from(room_id) else {
            log::error!("Invalid room_id for toggle_reaction: {}", room_id);
            return;
        };
        let Some(timeline) = self.timelines.get(&room_id) else {
            log::error!("No timeline subscription for room: {}", room_id);
            return;
        };
        let Ok(event_id) = matrix_sdk::ruma::OwnedEventId::try_from(event_id) else {
            log::error!("Invalid event_id for toggle_reaction: {}", event_id);
            return;
        };
        let item_id = TimelineEventItemId::EventId(event_id);
        if let Err(e) = timeline.toggle_reaction(&item_id, key).await {
            log::error!("Failed to toggle reaction: {:?}", e);
        }
    }

    /// Self-contained subscribe + paginate that can run in a spawned task
    /// without borrowing &mut self. The diff stream keeps the timeline alive.
    pub async fn subscribe_and_paginate(
        event_tx: mpsc::Sender<CoreEvent>,
        room: &Room,
        room_id: &OwnedRoomId,
        back_count: u16,
        media_sources: MediaSourceMap,
    ) {
        let Ok(timeline) = room.timeline().await else {
            log::error!("Failed to get timeline for room: {}", room_id);
            return;
        };
        let (initial_items, mut stream) = timeline.subscribe().await;

        let room_id_str = room_id.to_string();
        let messages: Vec<TimelineEntry> = initial_items
            .iter()
            .map(|item| timeline_item_to_entry(item, &media_sources))
            .collect();

        if !messages.is_empty() {
            let _ = event_tx.send(
                CoreEvent::Matrix(MatrixEvent::TimelineAppend(room_id_str.clone(), messages))
            ).await;
        }

        // Paginate before spawning the diff listener so initial history arrives first
        if let Err(e) = timeline.paginate_backwards(back_count).await {
            log::error!("Pagination error for room {}: {:?}", room_id, e);
        }

        // Spawn diff stream listener; moves `timeline` to keep it alive
        let tx = event_tx.clone();
        let rid = room_id_str.clone();
        let sources = media_sources.clone();
        tokio::spawn(async move {
            let _timeline = timeline; // prevent drop
            while let Some(diffs) = stream.next().await {
                for diff in diffs {
                    if let Some(entry) = map_diff(diff, &rid, &sources) {
                        let _ = tx.send(entry).await;
                    }
                }
            }
        });
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

                    TimelineEntryKind::Message(ChatMessageReceive {
                        id: event.event_id().map(|id| id.to_string()).unwrap_or_default(),
                        sender: event.sender().to_string(),
                        body: message.body().to_string(),
                        html_body,
                        media,
                        timestamp: ts as u128,
                        reactions,
                    })
                }
                MsgLikeKind::Redacted => TimelineEntryKind::Redacted,
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
        VectorDiff::Clear => Some(MatrixEvent::TimelineCleared(rid.to_owned())),
        VectorDiff::Reset { values } => {
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
