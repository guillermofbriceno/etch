import { writable, derived, get } from 'svelte/store';
import type { TimelineEntry } from '$lib/types';
import type { MatrixEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import { currentUser } from './user';
import { playSfx } from './sfx';
import { appFocused } from './eventRouter';

let lastNotifTime = 0;
const NOTIF_COOLDOWN_MS = 20_000;

// Hook for channels store to increment unread count (avoids circular import)
let _onUnreadMessage: ((roomId: string) => void) | null = null;
export function setOnUnreadMessage(fn: (roomId: string) => void) { _onUnreadMessage = fn; }

type ChannelWindow = {
    entries: TimelineEntry[];
    hasMore: boolean;
    loading: boolean;
};

const EMPTY_WINDOW: ChannelWindow = { entries: [], hasMore: true, loading: false };

// Single source of truth for the selected channel
export const activeChannelId = writable<string | null>(null);

const windows = writable<Record<string, ChannelWindow>>({});

// Active channel's window — what components subscribe to
export const activeWindow = derived(
    [windows, activeChannelId],
    ([$windows, $id]) => ($id ? $windows[$id] : null) ?? EMPTY_WINDOW
);

// Ensure a message window exists for a room (does not change selection)
function ensureWindowExists(room_id: string): void {
    windows.update(w => ({
        ...w,
        [room_id]: w[room_id] ?? { entries: [], hasMore: true, loading: false },
    }));
}

// Switch active channel, ensure its message window exists, and send a read receipt
export function setActiveChannel(id: string): void {
    activeChannelId.set(id);
    ensureWindowExists(id);

    // Send read receipt for the last message in the channel
    const win = get(windows)[id];
    if (win) {
        for (let i = win.entries.length - 1; i >= 0; i--) {
            const kind = win.entries[i].kind;
            if (typeof kind === 'object' && 'Message' in kind) {
                sendCoreCommand({
                    type: 'Matrix',
                    data: { type: 'SendReadReceipt', data: { room_id: id, event_id: kind.Message.id } },
                });
                break;
            }
        }
    }
}

export function loadOlder(): void {
    const id = get(activeChannelId);
    if (!id) return;

    windows.update(w => {
        const win = getWindow(w, id);
        return { ...w, [id]: { ...win, loading: true } };
    });

    sendCoreCommand({
        type: 'Matrix',
        data: { type: 'PaginateBackwards', data: { room_id: id } },
    });
}

export async function sendMessage(room_id: string, text: string, htmlBody: string | null = null, attachmentPath: string | null = null): Promise<void> {
    await sendCoreCommand({
        type: 'Matrix',
        data: {
            type: 'SendMessage',
            data: { room_id, text, html_body: htmlBody, attachment_path: attachmentPath },
        },
    });
}

export async function createDirectMessage(targetUserId: string): Promise<void> {
    await sendCoreCommand({
        type: 'Matrix',
        data: { type: 'CreateDirectMessage', data: { target_user_id: targetUserId } },
    });
}

export async function toggleReaction(eventId: string, key: string): Promise<void> {
    let roomId: string | null = null;
    activeChannelId.subscribe(id => { roomId = id; })();
    if (!roomId) return;
    await sendCoreCommand({
        type: 'Matrix',
        data: { type: 'ToggleReaction', data: { room_id: roomId, event_id: eventId, key } },
    });
}

// --- Handler called by eventRouter for Matrix timeline events ---

function getWindow(w: Record<string, ChannelWindow>, roomId: string): ChannelWindow {
    return w[roomId] ?? { entries: [], hasMore: true, loading: false };
}

export function handleMatrixEvent(me: MatrixEvent): void {
    switch (me.type) {
        case 'TimelineAppend': {
            const [roomId, entries] = me.data;
            windows.update(w => {
                const win = getWindow(w, roomId);
                return { ...w, [roomId]: { ...win, entries: [...win.entries, ...entries] } };
            });
            break;
        }
        case 'TimelinePushBack': {
            const [roomId, entry] = me.data;
            windows.update(w => {
                const win = getWindow(w, roomId);
                return { ...w, [roomId]: { ...win, entries: [...win.entries, entry] } };
            });

            // Notification sound: ring if message is from someone else AND
            // either the app is unfocused or the room isn't the active channel.
            const kind = entry.kind;
            if (typeof kind === 'object' && 'Message' in kind) {
                const sender = kind.Message.sender;
                const self = get(currentUser).matrixId;
                if (sender && sender !== self) {
                    const active = get(activeChannelId);
                    if (roomId !== active) {
                        _onUnreadMessage?.(roomId);
                    }
                    const now = Date.now();
                    if ((!appFocused || roomId !== active) && now - lastNotifTime >= NOTIF_COOLDOWN_MS) {
                        lastNotifTime = now;
                        playSfx('new_notif');
                    }
                }
            }
            break;
        }
        case 'TimelinePushFront': {
            const [roomId, entry] = me.data;
            windows.update(w => {
                const win = getWindow(w, roomId);
                return { ...w, [roomId]: { ...win, entries: [entry, ...win.entries] } };
            });
            break;
        }
        case 'TimelineInsert': {
            const [roomId, index, entry] = me.data;
            windows.update(w => {
                const win = w[roomId];
                if (!win) return w;
                const entries = [...win.entries];
                entries.splice(index, 0, entry);
                return { ...w, [roomId]: { ...win, entries } };
            });
            break;
        }
        case 'TimelineSet': {
            const [roomId, index, entry] = me.data;
            windows.update(w => {
                const win = w[roomId];
                if (!win) return w;
                const entries = [...win.entries];
                if (index >= 0 && index < entries.length) {
                    entries[index] = entry;
                }
                return { ...w, [roomId]: { ...win, entries } };
            });
            break;
        }
        case 'TimelineRemove': {
            const [roomId, index] = me.data;
            windows.update(w => {
                const win = w[roomId];
                if (!win) return w;
                const entries = [...win.entries];
                if (index >= 0 && index < entries.length) {
                    entries.splice(index, 1);
                }
                return { ...w, [roomId]: { ...win, entries } };
            });
            break;
        }
        case 'TimelineCleared': {
            const roomId = me.data;
            windows.update(w => ({
                ...w,
                [roomId]: { entries: [], hasMore: true, loading: false },
            }));
            break;
        }
        case 'TimelineReset': {
            const [roomId, entries] = me.data;
            windows.update(w => ({
                ...w,
                [roomId]: { entries, hasMore: true, loading: false },
            }));
            break;
        }
        case 'PaginationComplete': {
            const [roomId, hasMore] = me.data;
            windows.update(w => {
                const win = w[roomId];
                if (!win) return w;
                return { ...w, [roomId]: { ...win, loading: false, hasMore } };
            });
            break;
        }
    }
}
