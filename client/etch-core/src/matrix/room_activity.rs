use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use matrix_sdk::Room;
use matrix_sdk::event_handler::EventHandlerDropGuard;
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
use matrix_sdk::ruma::events::room::message::{OriginalSyncRoomMessageEvent, Relation};
use matrix_sdk::ruma::html::{HtmlSanitizerMode, RemoveReplyFallback};
use matrix_sdk::ruma::{EventId, MilliSecondsSinceUnixEpoch, OwnedUserId, UserId};
use matrix_sdk_ui::eyeball_im::Vector;
use matrix_sdk_ui::timeline::{MsgLikeKind, Timeline, TimelineItem, TimelineItemContent};
use tokio::sync::mpsc;

use crate::events::{CoreEvent, MatrixEvent};
use crate::matrix::name_colors::NameColorSender;
use crate::scripting::ScriptDispatcher;
use crate::task::AbortOnDrop;

/// A burst beyond this loses its pings, never the room's state, which the timeline carries.
const MESSAGE_QUEUE: usize = 32;

/// Long enough for a sync long poll to bring the receipt back, so a miss means the server did not.
pub(crate) const RECEIPT_CONFIRM_WAIT: Duration = Duration::from_secs(30);

/// Sends whether a room is unread and when it was last active as current values, and announces
/// each new message from someone else; it lives exactly as long as the room's subscription.
pub(crate) struct ActivityWatch {
    _handler: EventHandlerDropGuard,
    _task: AbortOnDrop,
}

pub(crate) struct ActivityFeed {
    pub event_tx: mpsc::Sender<CoreEvent>,
    pub dispatcher: Arc<ScriptDispatcher>,
    pub name_colors: NameColorSender,
}

struct IncomingMessage {
    sender: OwnedUserId,
    body: String,
}

impl ActivityWatch {
    pub(crate) fn start(room: &Room, timeline: Arc<Timeline>, feed: ActivityFeed) -> Self {
        let (tx, rx) = mpsc::channel(MESSAGE_QUEUE);
        let own_user_id = room.own_user_id().to_owned();
        let room_id = room.room_id().to_owned();
        let dropped = Arc::new(AtomicUsize::new(0));

        // The sync waits for its handlers, so this one only hands the message on.
        let handle = room.add_event_handler(move |mut ev: OriginalSyncRoomMessageEvent| {
            let (tx, own_user_id, room_id, dropped) = (tx.clone(), own_user_id.clone(), room_id.clone(), dropped.clone());
            async move {
                if ev.sender == own_user_id
                    || matches!(ev.content.relates_to, Some(Relation::Replacement(_)))
                {
                    return;
                }
                // As the timeline does, so a reply is not announced with the message it quotes.
                ev.content.sanitize(HtmlSanitizerMode::Compat, RemoveReplyFallback::Yes);
                let message = IncomingMessage { body: ev.content.body().to_owned(), sender: ev.sender };
                // One line per burst, not one per lost message.
                if tx.try_send(message).is_err() {
                    if dropped.fetch_add(1, Ordering::Relaxed) == 0 {
                        log::warn!("[activity] Dropping new message effects for {room_id}: its activity task is not keeping up");
                    }
                } else if let n @ 1.. = dropped.swap(0, Ordering::Relaxed) {
                    log::warn!("[activity] Dropped {n} new message effects for {room_id} before catching up");
                }
            }
        });

        Self {
            _handler: room.client().event_handler_drop_guard(handle),
            _task: AbortOnDrop::new(tokio::spawn(run(room.clone(), timeline, rx, feed))),
        }
    }
}

async fn run(
    room: Room,
    timeline: Arc<Timeline>,
    mut messages: mpsc::Receiver<IncomingMessage>,
    feed: ActivityFeed,
) {
    let (_, mut changes) = timeline.subscribe().await;
    let mut receipts = timeline.subscribe_own_user_read_receipts_changed().await;
    let mut last_activity = None;
    let mut sent = None;

    loop {
        let items = timeline.items().await;
        let read = timeline.latest_user_read_receipt_timeline_event_id(room.own_user_id()).await
            .and_then(|event_id| position_of(&items, &event_id));
        let shown = scan(&items, room.own_user_id());
        last_activity = shown.last_activity.or(last_activity);
        let current = (is_unread(shown.newest_confirmed.as_ref(), read), last_activity);
        if sent.map(|(unread, _)| unread) != Some(current.0) {
            log_unread_change(&room, current.0, &shown, read);
        }
        if sent != Some(current) {
            sent = Some(current);
            let (unread, last_activity) = current;
            let room_id = room.room_id().to_string();
            if !feed.send(MatrixEvent::RoomActivity { room_id, unread, last_activity }).await {
                return;
            }
        }

        tokio::select! {
            Some(_) = changes.next() => {}
            Some(()) = receipts.next() => {}
            Some(message) = messages.recv() => {
                if !feed.new_message(&room, message).await {
                    return;
                }
            }
            else => {
                log::warn!("[activity] Updates ended for room {}; its unread state is frozen until it is resubscribed", room.room_id());
                return;
            }
        }
    }
}

fn log_unread_change(room: &Room, unread: bool, shown: &Shown, read: Option<usize>) {
    let newest = shown.newest_confirmed.map(|m| m.position);
    log::debug!(
        "[activity] Room {} {}: newest confirmed message at {newest:?}{}, receipt at {read:?}, {} pending local echo(es)",
        room.room_id(),
        if unread { "unread" } else { "read" },
        if shown.newest_confirmed.is_some_and(|m| m.from_someone_else) { " from someone else" } else { "" },
        shown.pending,
    );
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReceiptOutcome {
    Confirmed,
    /// Sent, but no sync brought it back within the wait.
    Unconfirmed,
    AlreadyCovered,
    Failed,
}

/// The receipt is judged confirmed when the timeline sees the user's own receipt move.
pub(crate) async fn mark_read(timeline: &Timeline, room_id: &str, wait: Duration) -> ReceiptOutcome {
    let mut moved = timeline.subscribe_own_user_read_receipts_changed().await;
    match timeline.mark_as_read(ReceiptType::Read).await {
        Ok(false) => {
            log::debug!("[activity] Room {room_id} already read; no receipt sent");
            ReceiptOutcome::AlreadyCovered
        }
        Ok(true) => {
            if tokio::time::timeout(wait, moved.next()).await.is_ok() {
                log::debug!("[activity] Room {room_id} marked read");
                ReceiptOutcome::Confirmed
            } else {
                log::warn!("[activity] Read receipt for room {room_id} was sent but not confirmed by the server within {wait:?}");
                ReceiptOutcome::Unconfirmed
            }
        }
        Err(e) => {
            log::error!("Failed to mark {room_id} read: {e:?}");
            ReceiptOutcome::Failed
        }
    }
}

fn millis(ts: MilliSecondsSinceUnixEpoch) -> u64 {
    ts.0.into()
}

/// The newest message the server has confirmed, which is what a receipt can cover.
#[derive(Clone, Copy)]
struct NewestMessage {
    position: usize,
    from_someone_else: bool,
}

struct Shown {
    newest_confirmed: Option<NewestMessage>,
    /// Pending messages count, so sending moves a DM up.
    last_activity: Option<u64>,
    /// Only for the log: a message stuck sending is what once hid new ones.
    pending: usize,
}

fn scan(items: &Vector<Arc<TimelineItem>>, own_user_id: &UserId) -> Shown {
    let mut shown = Shown { newest_confirmed: None, last_activity: None, pending: 0 };
    for (position, item) in items.iter().enumerate().rev() {
        let Some(event) = item.as_event() else { continue };
        let TimelineItemContent::MsgLike(content) = event.content() else { continue };
        if matches!(content.kind, MsgLikeKind::Redacted) {
            continue;
        }
        shown.last_activity.get_or_insert(millis(event.timestamp()));
        if event.is_local_echo() {
            shown.pending += 1;
        } else {
            shown.newest_confirmed = Some(NewestMessage { position, from_someone_else: event.sender() != own_user_id });
            break;
        }
    }
    shown
}

fn position_of(items: &Vector<Arc<TimelineItem>>, event_id: &EventId) -> Option<usize> {
    items.iter().rposition(|item| item.as_event().and_then(|e| e.event_id()) == Some(event_id))
}

/// `read` is where the timeline places the user's newest receipt, public or private, which is
/// also what `Timeline::mark_as_read` compares against; it is `None` when that is not loaded.
fn is_unread(newest: Option<&NewestMessage>, read: Option<usize>) -> bool {
    newest.is_some_and(|m| m.from_someone_else && read.is_none_or(|read| read < m.position))
}

impl ActivityFeed {
    async fn new_message(&self, room: &Room, message: IncomingMessage) -> bool {
        let IncomingMessage { sender, body } = message;
        let display_name = match room.get_member_no_sync(&sender).await {
            Ok(Some(member)) => member.display_name().map(str::to_owned),
            _ => None,
        };
        let sender_name = display_name.unwrap_or_else(|| sender.localpart().to_owned());
        let room_id = room.room_id().to_string();

        self.dispatcher.fire("new_message", &[
            ("USER", &sender_name),
            ("MESSAGE", &body),
            ("ROOM", &room_id),
        ]);
        self.name_colors.activity(sender.as_str());
        log::debug!("[activity] Announced a new message in room {room_id}");
        self.send(MatrixEvent::NewMessage { room_id, sender_name, body }).await
    }

    async fn send(&self, event: MatrixEvent) -> bool {
        self.event_tx.send(CoreEvent::Matrix(event)).await.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_is_unread_until_the_receipt_reaches_its_newest_message_from_someone_else() {
        let theirs = Some(NewestMessage { position: 5, from_someone_else: true });
        let ours = Some(NewestMessage { position: 5, from_someone_else: false });
        let cases = [
            (None, None, false, "a room with no messages"),
            (ours, None, false, "the newest message is our own"),
            (theirs, None, true, "no receipt the timeline can place"),
            (theirs, Some(3), true, "the receipt is on an earlier event"),
            (theirs, Some(5), false, "the receipt is on it"),
            (theirs, Some(7), false, "the receipt is on something after it"),
        ];
        for (newest, read, unread, case) in cases {
            assert_eq!(is_unread(newest.as_ref(), read), unread, "{case}");
        }
    }
}
