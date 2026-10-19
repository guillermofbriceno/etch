import { writable, derived } from 'svelte/store';
import type { MatrixEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import { activeChannelId } from './activeChannel';
import { channels } from './channels';
import { appFocused } from './eventRouter';
import { matrixSessionLive } from './matrixConnection';
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

export const directMessages = derived(
    [channels, roomActivity],
    ([$channels, $activity]) => $channels
        .filter(c => c.etch_room_type === 'Dm')
        .sort((a, b) => ($activity[b.id]?.lastActivity ?? 0) - ($activity[a.id]?.lastActivity ?? 0)),
);

declareStores('derived', 'unreadRooms', 'directMessages');

export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type !== 'RoomActivity') return;
    const { room_id, unread, last_activity } = me.data;
    roomActivity.update(all => ({ ...all, [room_id]: { unread, lastActivity: last_activity } }));
}

/** While the session is live and the window focused, the open room is marked read when it is
 *  opened, when focus or the session returns, and when it becomes unread. */
export function initRoomActivity(): void {
    let previous: { id: string | null; ready: boolean; activity: RoomActivity | undefined } =
        { id: null, ready: false, activity: undefined };

    derived([activeChannelId, appFocused, matrixSessionLive, roomActivity], ([id, focused, live, all]) => ({
        id, ready: focused && live, activity: id ? all[id] : undefined,
    })).subscribe(current => {
        const { id, ready, activity } = current;
        const changed = id !== previous.id || !previous.ready
            || (activity !== previous.activity && activity?.unread === true);
        previous = current;
        if (id && ready && changed) {
            sendCoreCommand({ type: 'Matrix', data: { type: 'MarkRead', data: { room_id: id } } });
        }
    });
}
