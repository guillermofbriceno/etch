import { writable, derived } from 'svelte/store';
import type { MatrixEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import { activeChannelId } from './activeChannel';
import { appFocused } from './appFocus';
import { registerSessionStore, declareStores } from './session';

export type RoomActivity = { unread: boolean; lastActivity: number | null };

/** Written only by core's `RoomActivity`, so nothing that rebuilds a room can reset whether it is unread. */
export const roomActivity = writable<Record<string, RoomActivity>>({});

const clearRoomActivity = (): void => { roomActivity.set({}); };
registerSessionStore('matrix', 'roomActivity', clearRoomActivity);

/** The open room never shows a dot, even while its read receipt is on its way. */
export const unreadRooms = derived(
    [roomActivity, activeChannelId],
    ([$activity, $active]) => new Set(
        Object.entries($activity)
            .filter(([id, activity]) => activity.unread && id !== $active)
            .map(([id]) => id),
    ),
);

declareStores('derived', 'unreadRooms');

export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type !== 'RoomActivity') return;
    const { room_id, unread, last_activity } = me.data;
    roomActivity.update(all => ({ ...all, [room_id]: { unread, lastActivity: last_activity } }));
}

/** Tells core which room the user is looking at, which it keeps read; unfocused, it is none. */
export function initRoomActivity(): void {
    derived([activeChannelId, appFocused], ([id, focused]) => (focused ? id : null))
        .subscribe(room_id => sendCoreCommand({ type: 'Matrix', data: { type: 'ViewRoom', data: { room_id } } }));
}
