import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { listen } from '@tauri-apps/api/event';
import { resetStores } from './helpers';
import type { CoreEvent, SystemEvent } from '$lib/ipc';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

// Capture the core_event callback registered by initEventRouter.
type ListenCallback = (event: { payload: any }) => void;

function getListenCallback(eventName: string): ListenCallback {
    const calls = vi.mocked(listen).mock.calls;
    const match = calls.find(c => c[0] === eventName);
    if (!match) throw new Error(`No listen() call found for event "${eventName}"`);
    return match[1] as ListenCallback;
}

let routeCoreEvent: ListenCallback;

import { initEventRouter } from '../eventRouter';

vi.mocked(listen).mockClear();
initEventRouter();
routeCoreEvent = getListenCallback('core_event');

// Helpers

function fireServerReset(): void {
    routeCoreEvent({
        payload: { type: 'System', data: { type: 'ServerReset' } } satisfies CoreEvent,
    });
}

function fireMatrixEvent(data: any): void {
    routeCoreEvent({
        payload: { type: 'Matrix', data } satisfies CoreEvent,
    });
}

function makeRoom(id: string, name: string) {
    return {
        id,
        display_name: name,
        etch_room_type: 'Text' as const,
        channel_id: null,
        is_default: false,
        unread_count: 0,
        is_encrypted: false,
    };
}

function makeTimelineEntry(id: string, body: string) {
    return {
        sender: { display_name: 'Alice', avatar_url: null },
        kind: {
            Message: {
                id, sender: '@alice:s', body, html_body: null,
                media: null, timestamp: Date.now(), reactions: {},
            },
        },
    };
}

// -----------------------------------------------------------------------
// ServerReset clears session-bound stores
// -----------------------------------------------------------------------

describe('ServerReset clears session state', () => {
    beforeEach(() => {
        resetStores();
    });

    it('clears channels, messages, active channel, user, and server connection state', async () => {
        const { channels } = await import('../channels');
        const { activeWindow } = await import('../messages');
        const { activeChannelId } = await import('../activeChannel');
        const { currentUser } = await import('../user');
        const { mediaBaseUrl, passwordRequested } = await import('../servers');
        const { replyingTo } = await import('../compose');
        const { userVolumes } = await import('../userVolumes');

        // Populate session state as if connected to server A
        fireMatrixEvent({ type: 'ChannelList', data: [makeRoom('!r1:a', 'General')] });
        fireMatrixEvent({
            type: 'TimelinePushBack',
            data: ['!r1:a', makeTimelineEntry('$e1', 'hello from server A')],
        });
        fireMatrixEvent({
            type: 'CurrentUser',
            data: { username: 'alice', matrix_id: '@alice:a', display_name: 'Alice', avatar_url: 'mxc://a/pic' },
        });
        fireMatrixEvent({ type: 'HomeserverResolved', data: 'https://a.example.com' });

        // Set some compose and volume state
        const { setReply } = await import('../compose');
        setReply({ id: '$e1', sender: '@alice:a', body: 'hello', html_body: null, media: null, timestamp: Date.now(), reactions: {} });
        const { setUserVolume } = await import('../userVolumes');
        setUserVolume(1, -3.5);

        // Sanity: state is populated
        expect(get(channels).length).toBeGreaterThan(0);
        expect(get(activeChannelId)).not.toBeNull();
        expect(get(currentUser).username).toBe('alice');
        expect(get(mediaBaseUrl)).toBe('https://a.example.com');
        expect(get(replyingTo)).not.toBeNull();
        expect(get(userVolumes)).toHaveProperty('1');

        // Fire ServerReset
        fireServerReset();

        // All session-bound stores should be empty
        expect(get(channels)).toEqual([]);
        expect(get(activeChannelId)).toBeNull();
        expect(get(activeWindow).entries).toEqual([]);
        expect(get(currentUser)).toEqual({ username: '', matrixId: '', displayName: null, avatarUrl: null });
        expect(get(mediaBaseUrl)).toBeNull();
        expect(get(passwordRequested)).toBe(false);
        expect(get(replyingTo)).toBeNull();
        expect(get(userVolumes)).toEqual({});
    });
});

// -----------------------------------------------------------------------
// ServerReset preserves non-session stores
// -----------------------------------------------------------------------

describe('ServerReset preserves non-session state', () => {
    beforeEach(() => {
        resetStores();
    });

    it('does not clear bookmarks, overlay, audio, or voice settings', async () => {
        const { serverBookmarks, selectedBookmarkId } = await import('../servers');
        const { activeOverlay } = await import('../overlay');
        const { isMuted, isDeafened } = await import('../audio');
        const { transmissionMode } = await import('../voiceSettings');
        const { errorLog } = await import('../errors');

        // Set non-session state
        serverBookmarks.set([{
            id: 'bk1', label: 'My Server', address: 'example.com', port: 443,
            username: 'alice', auto_connect: true,
            mumble_host: null, mumble_port: null, mumble_username: null, mumble_password: null,
        }]);
        selectedBookmarkId.set('bk1');
        activeOverlay.set('settings');
        isMuted.set(true);
        isDeafened.set(true);
        transmissionMode.set('push_to_talk');
        errorLog.set([{ message: 'old error', target: 'test', timestamp: new Date() }]);

        fireServerReset();

        // Non-session state should survive
        expect(get(serverBookmarks)).toHaveLength(1);
        expect(get(serverBookmarks)[0].label).toBe('My Server');
        expect(get(selectedBookmarkId)).toBe('bk1');
        expect(get(activeOverlay)).toBe('settings');
        expect(get(isMuted)).toBe(true);
        expect(get(isDeafened)).toBe(true);
        expect(get(transmissionMode)).toBe('push_to_talk');
        expect(get(errorLog)).toHaveLength(1);
    });
});

// -----------------------------------------------------------------------
// Reconnect scenario: reset then repopulate
// -----------------------------------------------------------------------

describe('ServerReset prevents state interleaving on reconnect', () => {
    beforeEach(() => {
        resetStores();
    });

    it('new server data does not mix with old after reset', async () => {
        const { channels } = await import('../channels');
        const { activeWindow } = await import('../messages');
        const { activeChannelId } = await import('../activeChannel');
        const { currentUser } = await import('../user');
        const { mediaBaseUrl } = await import('../servers');

        // Session 1: connected to server A
        fireMatrixEvent({ type: 'ChannelList', data: [makeRoom('!r1:a', 'Server-A-General')] });
        fireMatrixEvent({
            type: 'TimelinePushBack',
            data: ['!r1:a', makeTimelineEntry('$a1', 'message from server A')],
        });
        fireMatrixEvent({
            type: 'CurrentUser',
            data: { username: 'alice', matrix_id: '@alice:a', display_name: 'Alice-A', avatar_url: null },
        });
        fireMatrixEvent({ type: 'HomeserverResolved', data: 'https://a.example.com' });

        expect(get(channels).some(c => c.display_name === 'Server-A-General')).toBe(true);

        // Reconnect: ServerReset fires, then server B data arrives
        fireServerReset();

        fireMatrixEvent({ type: 'ChannelList', data: [makeRoom('!r1:b', 'Server-B-Lobby')] });
        fireMatrixEvent({
            type: 'TimelinePushBack',
            data: ['!r1:b', makeTimelineEntry('$b1', 'message from server B')],
        });
        fireMatrixEvent({
            type: 'CurrentUser',
            data: { username: 'bob', matrix_id: '@bob:b', display_name: 'Bob-B', avatar_url: null },
        });
        fireMatrixEvent({ type: 'HomeserverResolved', data: 'https://b.example.com' });

        // Only server B data should be present
        const channelNames = get(channels).map(c => c.display_name);
        expect(channelNames).toContain('Server-B-Lobby');
        expect(channelNames).not.toContain('Server-A-General');

        // Active channel should point to server B's room
        expect(get(activeChannelId)).toBe('!r1:b');

        // Messages should be from server B only
        const entries = get(activeWindow).entries;
        const bodies = entries
            .map(e => typeof e.kind === 'object' && 'Message' in e.kind ? e.kind.Message.body : null)
            .filter(Boolean);
        expect(bodies).toContain('message from server B');
        expect(bodies).not.toContain('message from server A');

        // User and homeserver should be server B's
        expect(get(currentUser).username).toBe('bob');
        expect(get(currentUser).matrixId).toBe('@bob:b');
        expect(get(mediaBaseUrl)).toBe('https://b.example.com');
    });
});
