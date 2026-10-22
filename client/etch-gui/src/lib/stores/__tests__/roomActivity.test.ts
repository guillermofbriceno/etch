import { describe, it, expect, beforeAll, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { roomActivity, unreadRooms, handleMatrixEvent, initRoomActivity } from '../roomActivity';
import { channels, directMessages, handleMatrixEvent as channelsHandleMatrixEvent, initHiddenDms } from '../channels';
import { activeChannelId } from '../activeChannel';
import { appFocused } from '../appFocus';
import { resetStores } from './helpers';
import type { RoomInfo } from '$lib/types';

function room(id: string, type: RoomInfo['etch_room_type'] = 'Dm'): RoomInfo {
    return { id, display_name: id, etch_room_type: type, channel_id: null, is_default: false, is_encrypted: false, avatar_url: null };
}

function activity(roomId: string, unread: boolean, lastActivity: number | null = null) {
    handleMatrixEvent({ type: 'RoomActivity', data: { room_id: roomId, unread, last_activity: lastActivity } });
}

function viewed(): (string | null)[] {
    return vi.mocked(invoke).mock.calls
        .map(([, args]) => (args as any)?.command?.data)
        .filter(data => data?.type === 'ViewRoom')
        .map(data => data.data.room_id);
}

// The viewed room is a subscription that would stack if wired per test.
beforeAll(() => {
    initRoomActivity();
});

beforeEach(() => {
    resetStores();
    appFocused.set(true);
});

describe('room activity', () => {
    it('shows a dot for unread rooms except the open one', () => {
        activity('a', true);
        activity('b', true);
        activity('c', false);
        activeChannelId.set('b');

        expect([...get(unreadRooms)]).toEqual(['a']);
    });

    it('orders DMs by last activity, newest first, with unknown times last', () => {
        channels.set([room('old'), room('unknown'), room('t', 'Text'), room('new')]);
        activity('old', false, 1000);
        activity('new', false, 3000);

        expect(get(directMessages).map(r => r.id)).toEqual(['new', 'old', 'unknown']);
    });

    it('keeps a hidden DM\'s activity when DmCreated brings it back', () => {
        initHiddenDms(['dm1']);
        channelsHandleMatrixEvent({ type: 'ChannelList', data: [room('t', 'Text'), room('dm1')] });
        activity('dm1', true, 2000);

        channelsHandleMatrixEvent({ type: 'DmCreated', data: room('dm1') });
        activeChannelId.set('t');

        expect(get(roomActivity)['dm1']).toEqual({ unread: true, lastActivity: 2000 });
        expect(get(unreadRooms).has('dm1')).toBe(true);
    });
});

describe('the viewed room', () => {
    beforeEach(() => {
        activeChannelId.set('a');
        vi.mocked(invoke).mockClear();
    });

    it('is the open room while the window is focused, and none while it is not', () => {
        activeChannelId.set('b');
        appFocused.set(false);
        activeChannelId.set('c');
        appFocused.set(true);

        expect(viewed()).toEqual(['b', null, 'c']);
    });
});
