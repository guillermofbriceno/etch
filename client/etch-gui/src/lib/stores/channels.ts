import { writable, derived } from 'svelte/store';
import type { RoomInfo } from '$lib/types';
import type { MatrixEvent } from '$lib/ipc';
import { activeChannelId, setActiveChannel, setOnUnreadMessage } from '$lib/stores/messages';

export const channels = writable<RoomInfo[]>([]);

export { activeChannelId, setActiveChannel };

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

// Handler called by eventRouter for Matrix channel events
export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type === 'ChannelList') {
        channels.set(me.data);

        // Auto-select the first voice channel, falling back to first channel
        const firstVoice = me.data.find(c => c.etch_room_type === 'Voice');
        const defaultChannel = firstVoice ?? me.data[0];
        if (defaultChannel) {
            setActiveChannel(defaultChannel.id);
        }
    } else if (me.type === 'DmCreated') {
        channels.update(list => [...list, me.data]);
        setActiveChannel(me.data.id);
    }
}
