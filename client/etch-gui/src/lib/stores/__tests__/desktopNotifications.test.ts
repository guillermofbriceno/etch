import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { desktopNotifications } from '../desktopNotifications';
import { channels, handleMatrixEvent } from '../channels';
import { handleMumbleEvent } from '../voiceState';
import { appFocused } from '../eventRouter';
import { isDeafened } from '../audio';
import { deafenSuppressesNotifs } from '../voiceSettings';
import { currentUser } from '../user';
import { resetStores } from './helpers';
import type { RoomInfo } from '$lib/types';

function room(id: string, name: string, type: RoomInfo['etch_room_type']): RoomInfo {
    return { id, display_name: name, etch_room_type: type, channel_id: null, is_default: false, unread_count: 0, is_encrypted: false, avatar_url: null };
}

function receiveMessage(roomId: string, sender = '@alice:s') {
    handleMatrixEvent({
        type: 'TimelinePushBack',
        data: [roomId, {
            sender: { display_name: 'Alice', avatar_url: null },
            kind: { Message: { id: 'e1', sender, body: 'see <you> at 5', html_body: null, media: null, timestamp: 1000, edited: false, reactions: {} } },
        }],
    } as any);
}

function voiceUser(session_id: number, name: string, channel_id: number) {
    handleMumbleEvent({
        type: 'UserState',
        data: { session_id, name, display_name: null, avatar_url: null, channel_id, self_mute: false, self_deaf: false, hash: null },
    } as any);
}

function notifications() {
    return vi.mocked(invoke).mock.calls.filter(([command]) => command === 'show_notification').map(([, args]) => args);
}

beforeEach(() => {
    vi.useFakeTimers();
    resetStores();
    currentUser.set({ username: 'me', matrixId: '@me:s', displayName: null, avatarUrl: null });
    channels.set([room('t1', 'general', 'Text'), room('v1', 'Lounge', 'Voice'), room('dm1', 'Alice', 'Dm')]);

    handleMumbleEvent({ type: 'ChannelState', data: { id: 1, name: 'Lounge', parent: 0 } } as any);
    handleMumbleEvent({ type: 'LocalSession', data: 1 } as any);
    handleMumbleEvent({ type: 'ConnectionState', data: { type: 'Connected' } } as any);
    voiceUser(1, 'me', 1);
    vi.advanceTimersByTime(3000);

    appFocused.set(false);
    vi.mocked(invoke).mockClear();
});

afterEach(() => {
    appFocused.set(true);
    vi.useRealTimers();
});

describe('desktop notifications', () => {
    it.each([
        ['t1', 'Alice in #general'],
        ['v1', 'Alice in Lounge'],
        ['dm1', 'Alice'],
    ])('names the sender and the room for a message in %s', (roomId, title) => {
        receiveMessage(roomId);

        expect(notifications()).toEqual([{ title, body: 'see <you> at 5' }]);
    });

    it('says who joined and who left the local user\'s voice channel', () => {
        voiceUser(2, 'bob', 1);
        handleMumbleEvent({ type: 'UserRemoved', data: 2 } as any);

        expect(notifications()).toEqual([
            { title: 'bob', body: 'Joined Lounge' },
            { title: 'bob', body: 'Left Lounge' },
        ]);
    });

    it('stays quiet for the user\'s own message and for voice activity in another channel', () => {
        receiveMessage('t1', '@me:s');
        voiceUser(2, 'bob', 7);

        expect(notifications()).toEqual([]);
    });

    it.each([
        ['Etch is focused', () => appFocused.set(true)],
        ['the setting is off', () => desktopNotifications.set(false)],
        ['the user is deafened', () => isDeafened.set(true)],
    ])('shows nothing while %s', (_, arrange) => {
        arrange();

        receiveMessage('t1');
        voiceUser(2, 'bob', 1);

        expect(notifications()).toEqual([]);
    });

    it('lets messages, and only messages, through deafen when the user exempted them', () => {
        isDeafened.set(true);
        deafenSuppressesNotifs.set(false);

        receiveMessage('dm1');
        voiceUser(2, 'bob', 1);

        expect(notifications()).toEqual([{ title: 'Alice', body: 'see <you> at 5' }]);
    });

    it('is on until turned off, and a later start remembers the choice', async () => {
        localStorage.clear();
        vi.resetModules();
        const first = await import('../desktopNotifications');
        expect(get(first.desktopNotifications)).toBe(true);

        first.setDesktopNotifications(false);
        vi.resetModules();
        const second = await import('../desktopNotifications');

        expect(get(second.desktopNotifications)).toBe(false);
        localStorage.clear();
    });
});
