import { get } from 'svelte/store';
import type { MatrixEvent } from '$lib/ipc';
import { activeChannelId } from './activeChannel';
import { channels, surfaceRoom } from './channels';
import { notifyMessage } from './desktopNotifications';
import { appFocused } from './appFocus';
import { playSfx } from './sfx';

const SOUND_COOLDOWN_MS = 20_000;
let lastSound = 0;

/** Core decides what counts as a new message from someone else; this decides what it sets off. */
export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type !== 'NewMessage') return;
    const { room_id, sender_name, body } = me.data;
    const wasOpen = room_id === get(activeChannelId);

    surfaceRoom(room_id);
    notifyMessage(get(channels).find(c => c.id === room_id), sender_name, body);

    const now = Date.now();
    if ((!get(appFocused) || !wasOpen) && now - lastSound >= SOUND_COOLDOWN_MS) {
        lastSound = now;
        playSfx('new_notif');
    }
}
