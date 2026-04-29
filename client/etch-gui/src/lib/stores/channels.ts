import { writable, derived, get } from 'svelte/store';
import type { RoomInfo } from '$lib/types';
import type { MatrixEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import { activeChannelId, setActiveChannel, setOnUnreadMessage } from '$lib/stores/messages';
import { currentUser } from './user';

export const channels = writable<RoomInfo[]>([]);

export { activeChannelId, setActiveChannel };

// --- Hidden DM state (Etch-level, not Matrix) ---

const hiddenDmIds = writable<Set<string>>(new Set());
const hiddenDmInfos = writable<Map<string, RoomInfo>>(new Map());

export function initHiddenDms(ids: string[]): void {
    hiddenDmIds.set(new Set(ids));
}

export function hideDm(roomId: string): void {
    // Stash room info for potential unhide later
    const room = get(channels).find(c => c.id === roomId);
    if (room) {
        hiddenDmInfos.update(m => { m.set(roomId, room); return new Map(m); });
    }
    hiddenDmIds.update(s => { s.add(roomId); return new Set(s); });
    channels.update(list => list.filter(c => c.id !== roomId));

    if (get(activeChannelId) === roomId) {
        const remaining = get(channels);
        const fallback = remaining.find(c => c.etch_room_type === 'Voice') ?? remaining[0];
        if (fallback) setActiveChannel(fallback.id);
    }

    sendCoreCommand({ type: 'System', data: { type: 'HideDm', data: { room_id: roomId } } });
}

export function unhideDm(roomId: string): void {
    if (!get(hiddenDmIds).has(roomId)) return;

    const info = get(hiddenDmInfos).get(roomId);
    hiddenDmIds.update(s => { s.delete(roomId); return new Set(s); });
    hiddenDmInfos.update(m => { m.delete(roomId); return new Map(m); });

    if (info) {
        channels.update(list => {
            if (list.some(c => c.id === roomId)) return list;
            return [...list, { ...info, unread_count: 1 }];
        });
    }

    sendCoreCommand({ type: 'System', data: { type: 'UnhideDm', data: { room_id: roomId } } });
}

// --- Unread / active channel bookkeeping ---

// Clear unread count when switching to a channel
activeChannelId.subscribe(id => {
    if (!id) return;
    channels.update(list =>
        list.map(c => c.id === id ? { ...c, unread_count: 0 } : c)
    );
});

// Increment unread count when a message arrives in a non-active channel
setOnUnreadMessage((roomId: string) => {
    channels.update(list =>
        list.map(c => c.id === roomId ? { ...c, unread_count: c.unread_count + 1 } : c)
    );
});

export const activeChannel = derived(
    [channels, activeChannelId],
    ([$channels, $id]) => $channels.find(c => c.id === $id) ?? null
);

// --- Event handler called by eventRouter ---

export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type === 'ChannelList') {
        const hidden = get(hiddenDmIds);
        const visible: RoomInfo[] = [];
        const stash = new Map(get(hiddenDmInfos));

        for (const room of me.data) {
            if (room.etch_room_type === 'Dm' && hidden.has(room.id)) {
                stash.set(room.id, room);
            } else {
                visible.push(room);
            }
        }

        hiddenDmInfos.set(stash);
        channels.set(visible);

        // Auto-select the first voice channel, falling back to first channel
        const firstVoice = visible.find(c => c.etch_room_type === 'Voice');
        const defaultChannel = firstVoice ?? visible[0];
        if (defaultChannel) {
            setActiveChannel(defaultChannel.id);
        }
    } else if (me.type === 'DmCreated') {
        // If this was a hidden DM being reused, unhide it
        if (get(hiddenDmIds).has(me.data.id)) {
            hiddenDmIds.update(s => { s.delete(me.data.id); return new Set(s); });
            hiddenDmInfos.update(m => { m.delete(me.data.id); return new Map(m); });
            sendCoreCommand({ type: 'System', data: { type: 'UnhideDm', data: { room_id: me.data.id } } });
        }

        channels.update(list => {
            if (list.some(c => c.id === me.data.id)) return list;
            return [...list, me.data];
        });
        setActiveChannel(me.data.id);
    } else if (me.type === 'TimelinePushBack') {
        // Auto-unhide hidden DMs when the other person messages
        const [roomId, entry] = me.data;
        if (get(hiddenDmIds).has(roomId)) {
            const kind = entry.kind;
            if (typeof kind === 'object' && 'Message' in kind) {
                const sender = kind.Message.sender;
                if (sender && sender !== get(currentUser).matrixId) {
                    unhideDm(roomId);
                }
            }
        }
    }
}
