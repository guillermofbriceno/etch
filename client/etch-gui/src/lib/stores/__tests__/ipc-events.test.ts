import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { listen } from '@tauri-apps/api/event';
import { resetStores } from './helpers';
import type { CoreEvent } from '$lib/ipc';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

// We need to capture the callbacks that initEventRouter registers.
// Since listen is mocked, we extract them from the mock call history.
type ListenCallback = (event: { payload: any }) => void;

function getListenCallback(eventName: string): ListenCallback {
    const calls = vi.mocked(listen).mock.calls;
    const match = calls.find(c => c[0] === eventName);
    if (!match) throw new Error(`No listen() call found for event "${eventName}"`);
    return match[1] as ListenCallback;
}

// Call initEventRouter once for the suite -- it registers listen() callbacks
// that we'll reuse across all tests.
let routeCoreEvent: ListenCallback;
let routeFocus: ListenCallback;
let routeBlur: ListenCallback;

beforeEach(() => {
    resetStores();
});

// Initialize the event router and capture callbacks.
// This must happen after the mocks are in place but before tests run.
import { initEventRouter, appFocused } from '../eventRouter';

vi.mocked(listen).mockClear();
initEventRouter();
routeCoreEvent = getListenCallback('core_event');
routeFocus = getListenCallback('tauri://focus');
routeBlur = getListenCallback('tauri://blur');

// -----------------------------------------------------------------------
// Focus tracking
// -----------------------------------------------------------------------

describe('focus tracking', () => {
    it('sets appFocused to true on tauri://focus', () => {
        appFocused.set(false);

        routeFocus({ payload: undefined });

        expect(get(appFocused)).toBe(true);
    });

    it('sets appFocused to false on tauri://blur', () => {
        appFocused.set(true);

        routeBlur({ payload: undefined });

        expect(get(appFocused)).toBe(false);
    });
});

// -----------------------------------------------------------------------
// Matrix event routing
// -----------------------------------------------------------------------

describe('Matrix event routing', () => {
    it('routes ChannelList to channels store', async () => {
        const { channels } = await import('../channels');
        const rooms = [
            { id: 't1', display_name: 'General', etch_room_type: 'Text' as const, channel_id: null, is_default: false, unread_count: 0, is_encrypted: false, avatar_url: null },
        ];

        routeCoreEvent({
            payload: { type: 'Matrix', data: { type: 'ChannelList', data: rooms } } satisfies CoreEvent,
        });

        expect(get(channels)).toHaveLength(1);
        expect(get(channels)[0].display_name).toBe('General');
    });

    it('routes CurrentUser to user store', async () => {
        const { currentUser } = await import('../user');

        routeCoreEvent({
            payload: {
                type: 'Matrix',
                data: {
                    type: 'CurrentUser',
                    data: { username: 'nyx', matrix_id: '@nyx:etch.gg', display_name: 'Nyx', avatar_url: null },
                },
            } satisfies CoreEvent,
        });

        const user = get(currentUser);
        expect(user.username).toBe('nyx');
        expect(user.matrixId).toBe('@nyx:etch.gg');
        expect(user.displayName).toBe('Nyx');
    });

    it('routes TimelinePushBack to messages store', async () => {
        const { activeWindow } = await import('../messages');
        const { activeChannelId } = await import('../activeChannel');
        activeChannelId.set('room1');

        routeCoreEvent({
            payload: {
                type: 'Matrix',
                data: {
                    type: 'TimelinePushBack',
                    data: ['room1', {
                        sender: { display_name: 'A', avatar_url: null },
                        kind: { Message: { id: '$e1', sender: '@a:s', body: 'routed', html_body: null, media: null, timestamp: Date.now(), reactions: {} } },
                    }],
                },
            } satisfies CoreEvent,
        });

        const entries = get(activeWindow).entries;
        expect(entries.length).toBeGreaterThan(0);
        const kind = entries[entries.length - 1].kind;
        expect(typeof kind === 'object' && 'Message' in kind && kind.Message.body).toBe('routed');
    });

    it('routes PasswordRequest to servers store', async () => {
        const { passwordRequested } = await import('../servers');

        routeCoreEvent({
            payload: { type: 'Matrix', data: { type: 'PasswordRequest' } } satisfies CoreEvent,
        });

        expect(get(passwordRequested)).toBe(true);
    });

    it('routes HomeserverResolved to servers store', async () => {
        const { mediaBaseUrl } = await import('../servers');

        routeCoreEvent({
            payload: {
                type: 'Matrix',
                data: { type: 'HomeserverResolved', data: 'https://matrix.etch.gg' },
            } satisfies CoreEvent,
        });

        expect(get(mediaBaseUrl)).toBe('https://matrix.etch.gg');
    });
});

// -----------------------------------------------------------------------
// Mumble event routing
// -----------------------------------------------------------------------

describe('Mumble event routing', () => {
    it('routes ChannelState to voiceChannels store', async () => {
        const { voiceChannels } = await import('../voiceState');

        routeCoreEvent({
            payload: {
                type: 'Mumble',
                data: { type: 'ChannelState', data: { id: 1, name: 'Lounge', parent: 0 } },
            } satisfies CoreEvent,
        });

        expect(get(voiceChannels).get(1)).toEqual({ id: 1, name: 'Lounge', parent: 0 });
    });

    it('routes UserState to voiceUsers store', async () => {
        const { voiceUsers } = await import('../voiceState');

        routeCoreEvent({
            payload: {
                type: 'Mumble',
                data: {
                    type: 'UserState',
                    data: {
                        session_id: 1, name: 'alice', display_name: 'Alice',
                        avatar_url: null, channel_id: 1,
                        self_mute: false, self_deaf: false, hash: null,
                    },
                },
            } satisfies CoreEvent,
        });

        expect(get(voiceUsers).get(1)?.name).toBe('alice');
    });

    it('routes UserTalking to talkingUsers store', async () => {
        const { talkingUsers } = await import('../voiceState');

        routeCoreEvent({
            payload: {
                type: 'Mumble',
                data: { type: 'UserTalking', data: { session_id: 5, talking: true } },
            } satisfies CoreEvent,
        });

        expect(get(talkingUsers).has(5)).toBe(true);
    });

    it('routes ConnectionState to mumbleStatus store', async () => {
        const { mumbleStatus } = await import('../voiceState');

        routeCoreEvent({
            payload: {
                type: 'Mumble',
                data: { type: 'ConnectionState', data: { type: 'Connected' } },
            } satisfies CoreEvent,
        });

        expect(get(mumbleStatus)).toBe('connected');
    });
});

// -----------------------------------------------------------------------
// System event routing
// -----------------------------------------------------------------------

describe('System event routing', () => {
    it('routes LogError to errorLog store', async () => {
        const { errorLog } = await import('../errors');

        routeCoreEvent({
            payload: {
                type: 'System',
                data: { type: 'LogError', data: { message: 'Something broke', target: 'etch_core' } },
            } satisfies CoreEvent,
        });

        const log = get(errorLog);
        expect(log).toHaveLength(1);
        expect(log[0].message).toBe('Something broke');
        expect(log[0].target).toBe('etch_core');
    });

    it('routes SettingsLoaded to servers store', async () => {
        const { serverBookmarks } = await import('../servers');

        const bookmark = {
            id: 'bk1', label: 'Test', address: 'example.com', port: 443,
            username: 'nyx', auto_connect: false,
            mumble_host: null, mumble_port: null, mumble_username: null, mumble_password: null,
        };

        routeCoreEvent({
            payload: {
                type: 'System',
                data: {
                    type: 'SettingsLoaded',
                    data: {
                        bookmarks: [bookmark],
                        transmission_mode: null,
                        vad_threshold: null,
                        voice_hold: null,
                        use_mumble_settings: null,
                        deafen_suppresses_notifs: null,
                        hidden_dms: [],
                    },
                },
            } satisfies CoreEvent,
        });

        expect(get(serverBookmarks)).toHaveLength(1);
        expect(get(serverBookmarks)[0].label).toBe('Test');
    });

    it('routes UserProfileChanged to user store', async () => {
        const { currentUser } = await import('../user');
        currentUser.set({ username: 'nyx', matrixId: '@nyx:etch.gg', displayName: 'Old', avatarUrl: null });

        routeCoreEvent({
            payload: {
                type: 'System',
                data: { type: 'UserProfileChanged', data: { username: 'nyx', display_name: 'Nyx!', avatar_url: 'http://pic' } },
            } satisfies CoreEvent,
        });

        expect(get(currentUser).displayName).toBe('Nyx!');
        expect(get(currentUser).avatarUrl).toBe('http://pic');
    });

    it('routes UserProfileChanged to voiceState store', async () => {
        const { voiceUsers, handleMumbleEvent } = await import('../voiceState');

        // Add a user to voice first
        handleMumbleEvent({
            type: 'UserState',
            data: {
                session_id: 1, name: 'nyx', display_name: 'Old',
                avatar_url: null, channel_id: 1,
                self_mute: false, self_deaf: false, hash: null,
            },
        } as any);

        routeCoreEvent({
            payload: {
                type: 'System',
                data: { type: 'UserProfileChanged', data: { username: 'nyx', display_name: 'New Nyx', avatar_url: 'http://avatar' } },
            } satisfies CoreEvent,
        });

        expect(get(voiceUsers).get(1)?.display_name).toBe('New Nyx');
        expect(get(voiceUsers).get(1)?.avatar_url).toBe('http://avatar');
    });
});
