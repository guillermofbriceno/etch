import { writable, derived, get } from 'svelte/store';
import type { TimelineEntry } from '$lib/types';
import type { MatrixEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import { probeMediaInfo, type AttachedFile } from '$lib/attachments';
import { activeChannelId } from './activeChannel';
import { registerSessionStore, declareStores } from './session';
import { showToast } from './errors';

type ChannelWindow = {
    entries: TimelineEntry[];
    hasMore: boolean;
    loading: boolean;
};

const EMPTY_WINDOW: ChannelWindow = { entries: [], hasMore: true, loading: false };

const messageWindows = writable<Record<string, ChannelWindow>>({});

// Active channel's window — what components subscribe to
export const activeWindow = derived(
    [messageWindows, activeChannelId],
    ([$messageWindows, $id]) => ($id ? $messageWindows[$id] : null) ?? EMPTY_WINDOW
);

declareStores('derived', 'activeWindow');

// Ensure a message window exists for a room (does not change selection)
function ensureWindowExists(room_id: string): void {
    messageWindows.update(w => ({
        ...w,
        [room_id]: w[room_id] ?? { entries: [], hasMore: true, loading: false },
    }));
}

// Switch active channel and ensure its message window exists
export function setActiveChannel(id: string): void {
    activeChannelId.set(id);
    ensureWindowExists(id);
}

export function loadOlder(): void {
    const id = get(activeChannelId);
    if (!id) return;

    messageWindows.update(w => {
        const win = getWindow(w, id);
        return { ...w, [id]: { ...win, loading: true } };
    });

    sendCoreCommand({
        type: 'Matrix',
        data: { type: 'PaginateBackwards', data: { room_id: id } },
    });
}

export async function sendMessage(room_id: string, text: string, htmlBody: string | null = null): Promise<void> {
    await sendCoreCommand({
        type: 'Matrix',
        data: { type: 'SendMessage', data: { room_id, text, html_body: htmlBody } },
    });
}

/** Core owns the file from here on, including discarding it; only a throw means it never got there. */
export async function sendAttachment(roomId: string, file: AttachedFile, compress: boolean): Promise<void> {
    const mediaInfo = await probeMediaInfo(file.path, file.size, file.inspection.mimetype);
    await sendCoreCommand({
        type: 'Matrix',
        data: { type: 'SendAttachment', data: { room_id: roomId, path: file.path, compress, media_info: mediaInfo } },
    });
}

export async function createDirectMessage(targetUserId: string): Promise<void> {
    await sendCoreCommand({
        type: 'Matrix',
        data: { type: 'CreateDirectMessage', data: { target_user_id: targetUserId } },
    });
}

export async function editMessage(roomId: string, eventId: string, text: string, htmlBody: string | null): Promise<void> {
    await sendCoreCommand({
        type: 'Matrix',
        data: { type: 'EditMessage', data: { room_id: roomId, event_id: eventId, text, html_body: htmlBody } },
    });
}

export async function redactMessage(roomId: string, eventId: string): Promise<void> {
    await sendCoreCommand({
        type: 'Matrix',
        data: { type: 'RedactMessage', data: { room_id: roomId, event_id: eventId } },
    });
}

export async function toggleReaction(eventId: string, key: string): Promise<void> {
    const roomId = get(activeChannelId);
    if (!roomId) return;
    await sendCoreCommand({
        type: 'Matrix',
        data: { type: 'ToggleReaction', data: { room_id: roomId, event_id: eventId, key } },
    });
}

export function resetMessages(): void {
    messageWindows.set({});
}

registerSessionStore('matrix', 'messageWindows', resetMessages);

// --- Handler called by eventRouter for Matrix timeline events ---

function getWindow(w: Record<string, ChannelWindow>, roomId: string): ChannelWindow {
    return w[roomId] ?? { entries: [], hasMore: true, loading: false };
}

export function handleMatrixEvent(me: MatrixEvent): void {
    switch (me.type) {
        case 'TimelineAppend': {
            const [roomId, entries] = me.data;
            messageWindows.update(w => {
                const win = getWindow(w, roomId);
                return { ...w, [roomId]: { ...win, entries: [...win.entries, ...entries] } };
            });
            break;
        }
        case 'TimelinePushBack': {
            const [roomId, entry] = me.data;
            messageWindows.update(w => {
                const win = getWindow(w, roomId);
                return { ...w, [roomId]: { ...win, entries: [...win.entries, entry] } };
            });
            break;
        }
        case 'TimelinePushFront': {
            const [roomId, entry] = me.data;
            messageWindows.update(w => {
                const win = getWindow(w, roomId);
                return { ...w, [roomId]: { ...win, entries: [entry, ...win.entries] } };
            });
            break;
        }
        case 'TimelineInsert': {
            const [roomId, index, entry] = me.data;
            messageWindows.update(w => {
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
            messageWindows.update(w => {
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
            messageWindows.update(w => {
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
            messageWindows.update(w => ({
                ...w,
                [roomId]: { entries: [], hasMore: true, loading: false },
            }));
            break;
        }
        case 'TimelineReset': {
            const [roomId, entries] = me.data;
            messageWindows.update(w => ({
                ...w,
                [roomId]: { entries, hasMore: true, loading: false },
            }));
            break;
        }
        case 'PaginationComplete': {
            const [roomId, hasMore] = me.data;
            messageWindows.update(w => {
                const win = w[roomId];
                if (!win) return w;
                return { ...w, [roomId]: { ...win, loading: false, hasMore } };
            });
            break;
        }
        case 'AttachmentFailed':
            showToast(`Couldn't send ${me.data.file_name}: ${me.data.reason}`);
            break;
        case 'SendFailed':
            showToast(`Couldn't send a message: ${me.data.reason}`);
            break;
    }
}
