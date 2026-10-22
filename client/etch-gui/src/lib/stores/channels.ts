import { writable, derived, get } from 'svelte/store';
import type { RoomInfo } from '$lib/types';
import type { MatrixEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import { activeChannelId } from './activeChannel';
import { setActiveChannel } from './messages';
import { isComposing } from './compose';
import { followNewMessages } from './followMessages';
import { roomActivity } from './roomActivity';
import { registerSessionStore, declareStores } from './session';

export const channels = writable<RoomInfo[]>([]);

// --- Hidden DM state (Etch-level, not Matrix) ---

// SettingsLoaded does not fire again on reconnect, so this must survive a reset.
const hiddenDmIds = writable<Set<string>>(new Set());

declareStores('device', 'hiddenDmIds');

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
            return [...list, info];
        });
    }

    sendCoreCommand({ type: 'System', data: { type: 'UnhideDm', data: { room_id: roomId } } });
}

const clearChannels = (): void => { channels.set([]); };
const clearHiddenDmInfos = (): void => { hiddenDmInfos.set(new Map()); };

registerSessionStore('matrix', 'channels', clearChannels);
registerSessionStore('matrix', 'hiddenDmInfos', clearHiddenDmInfos);

// --- A new message from someone else ---

// A draft travels with the composer, so following mid-message would send it to the wrong room.
function followMessage(roomId: string): void {
    if (!get(followNewMessages) || isComposing()) return;
    if (!get(channels).some(c => c.id === roomId)) return;
    setActiveChannel(roomId);
}

/** Brings back a hidden DM, and opens the room if the user follows new messages. */
export function surfaceRoom(roomId: string): void {
    unhideDm(roomId);
    if (roomId !== get(activeChannelId)) followMessage(roomId);
}

export const activeChannel = derived(
    [channels, activeChannelId],
    ([$channels, $id]) => $channels.find(c => c.id === $id) ?? null
);

declareStores('derived', 'activeChannel');

const byChannelId = (a: RoomInfo, b: RoomInfo) => (a.channel_id ?? 999) - (b.channel_id ?? 999);

export const voiceRooms = derived(channels, $channels => $channels.filter(c => c.etch_room_type === 'Voice').sort(byChannelId));

export const textRooms = derived(channels, $channels => $channels.filter(c => c.etch_room_type === 'Text').sort(byChannelId));

export const directMessages = derived(
    [channels, roomActivity],
    ([$channels, $activity]) => $channels
        .filter(c => c.etch_room_type === 'Dm')
        .sort((a, b) => ($activity[b.id]?.lastActivity ?? 0) - ($activity[a.id]?.lastActivity ?? 0)),
);

declareStores('derived', 'voiceRooms', 'textRooms', 'directMessages');

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
    }
}
