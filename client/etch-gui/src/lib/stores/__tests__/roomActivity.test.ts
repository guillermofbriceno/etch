import { describe, it, expect, beforeAll, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { roomActivity, unreadRooms, directMessages, handleMatrixEvent, initRoomActivity } from '../roomActivity';
import { channels, handleMatrixEvent as channelsHandleMatrixEvent, initHiddenDms } from '../channels';
import { activeChannelId } from '../activeChannel';
import { appFocused } from '../eventRouter';
import { matrixStatus } from '../matrixConnection';
import { resetStores } from './helpers';
import type { RoomInfo } from '$lib/types';

function room(id: string, type: RoomInfo['etch_room_type'] = 'Dm'): RoomInfo {
    return { id, display_name: id, etch_room_type: type, channel_id: null, is_default: false, is_encrypted: false, avatar_url: null };
}

function activity(roomId: string, unread: boolean, lastActivity: number | null = null) {
    handleMatrixEvent({ type: 'RoomActivity', data: { room_id: roomId, unread, last_activity: lastActivity } });
}

function markReads(): string[] {
    return vi.mocked(invoke).mock.calls
        .map(([, args]) => (args as any)?.command?.data)
        .filter(data => data?.type === 'MarkRead')
        .map(data => data.data.room_id);
}

// The read receipt triggers are a subscription that would stack if wired per test.
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

describe('marking the open room read', () => {
    beforeEach(() => {
        matrixStatus.set('connected');
        activeChannelId.set('a');
        vi.mocked(invoke).mockClear();
    });

    it('marks a room read when it is opened, and again when it becomes unread', () => {
        activeChannelId.set('b');
        activity('b', true);
        activity('a', true);

        expect(markReads()).toEqual(['b', 'b']);
    });

    it.each([
        ['focus', () => appFocused.set(false), () => appFocused.set(true)],
        ['a live session', () => matrixStatus.set('disconnected'), () => matrixStatus.set('connected')],
    ])('waits for %s, then marks the open room read', (_, lose, regain) => {
        lose();
        activity('a', true);
        activeChannelId.set('b');
        expect(markReads()).toEqual([]);

        regain();

        expect(markReads()).toEqual(['b']);
    });
});
