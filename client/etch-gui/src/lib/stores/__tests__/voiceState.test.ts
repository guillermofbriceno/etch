import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import {
    voiceChannels, voiceUsers, talkingUsers, mumbleStatus, voiceConnected,
    usersByChannel, handleMumbleEvent, handleSystemEvent,
} from '../voiceState';
import { isMuted, isDeafened } from '../audio';
import { userVolumes } from '../userVolumes';
import { resetStores } from './helpers';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

beforeEach(() => {
    resetStores();
});

function addUser(session_id: number, name: string, channel_id: number) {
    handleMumbleEvent({
        type: 'UserState',
        data: {
            session_id, name, display_name: null, avatar_url: null,
            channel_id, self_mute: false, self_deaf: false, hash: null,
        },
    } as any);
}

describe('handleMumbleEvent', () => {
    describe('ChannelState', () => {
        it('adds a new voice channel', () => {
            handleMumbleEvent({
                type: 'ChannelState',
                data: { id: 1, name: 'General', parent: 0 },
            } as any);

            expect(get(voiceChannels).get(1)).toEqual({ id: 1, name: 'General', parent: 0 });
        });

        it('updates an existing voice channel', () => {
            handleMumbleEvent({ type: 'ChannelState', data: { id: 1, name: 'General', parent: 0 } } as any);
            handleMumbleEvent({ type: 'ChannelState', data: { id: 1, name: 'Lounge', parent: 0 } } as any);

            expect(get(voiceChannels).get(1)?.name).toBe('Lounge');
        });

        it('adds multiple channels independently', () => {
            handleMumbleEvent({ type: 'ChannelState', data: { id: 1, name: 'General', parent: 0 } } as any);
            handleMumbleEvent({ type: 'ChannelState', data: { id: 2, name: 'Lounge', parent: 0 } } as any);
            handleMumbleEvent({ type: 'ChannelState', data: { id: 3, name: 'AFK', parent: 1 } } as any);

            expect(get(voiceChannels).size).toBe(3);
            expect(get(voiceChannels).get(3)?.parent).toBe(1);
        });

        it('updates only the specified channel, leaving others unchanged', () => {
            handleMumbleEvent({ type: 'ChannelState', data: { id: 1, name: 'General', parent: 0 } } as any);
            handleMumbleEvent({ type: 'ChannelState', data: { id: 2, name: 'Lounge', parent: 0 } } as any);
            handleMumbleEvent({ type: 'ChannelState', data: { id: 1, name: 'Renamed', parent: 0 } } as any);

            expect(get(voiceChannels).get(1)?.name).toBe('Renamed');
            expect(get(voiceChannels).get(2)?.name).toBe('Lounge');
        });
    });

    describe('ChannelRemoved', () => {
        it('removes a voice channel by id', () => {
            handleMumbleEvent({ type: 'ChannelState', data: { id: 1, name: 'General', parent: 0 } } as any);
            handleMumbleEvent({ type: 'ChannelRemoved', data: 1 } as any);

            expect(get(voiceChannels).has(1)).toBe(false);
        });

        it('does not throw when removing a nonexistent channel', () => {
            handleMumbleEvent({ type: 'ChannelRemoved', data: 999 } as any);

            expect(get(voiceChannels).size).toBe(0);
        });

        it('only removes the targeted channel', () => {
            handleMumbleEvent({ type: 'ChannelState', data: { id: 1, name: 'A', parent: 0 } } as any);
            handleMumbleEvent({ type: 'ChannelState', data: { id: 2, name: 'B', parent: 0 } } as any);
            handleMumbleEvent({ type: 'ChannelRemoved', data: 1 } as any);

            expect(get(voiceChannels).has(1)).toBe(false);
            expect(get(voiceChannels).has(2)).toBe(true);
        });
    });

    describe('UserState', () => {
        it('adds a new user', () => {
            addUser(10, 'alice', 1);

            const user = get(voiceUsers).get(10);
            expect(user).toBeDefined();
            expect(user?.name).toBe('alice');
            expect(user?.channel_id).toBe(1);
            expect(user?.muted).toBe(false);
            expect(user?.deafened).toBe(false);
        });

        it('merges partial updates into an existing user', () => {
            addUser(10, 'alice', 1);

            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 10, name: null, display_name: null,
                    avatar_url: null, channel_id: 2,
                    self_mute: null, self_deaf: null, hash: null,
                },
            } as any);

            const user = get(voiceUsers).get(10);
            expect(user?.name).toBe('alice');
            expect(user?.channel_id).toBe(2);
        });

        it('preserves all fields when update has all-null values', () => {
            addUser(10, 'alice', 1);

            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 10, name: null, display_name: null,
                    avatar_url: null, channel_id: null,
                    self_mute: null, self_deaf: null, hash: null,
                },
            } as any);

            const user = get(voiceUsers).get(10);
            expect(user?.name).toBe('alice');
            expect(user?.channel_id).toBe(1);
            expect(user?.muted).toBe(false);
            expect(user?.deafened).toBe(false);
        });

        it('defaults name to empty string for a new user with null name', () => {
            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 10, name: null, display_name: null,
                    avatar_url: null, channel_id: 1,
                    self_mute: false, self_deaf: false, hash: null,
                },
            } as any);

            expect(get(voiceUsers).get(10)?.name).toBe('');
        });

        it('defaults channel_id to 0 for a new user with null channel_id', () => {
            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 10, name: 'alice', display_name: null,
                    avatar_url: null, channel_id: null,
                    self_mute: false, self_deaf: false, hash: null,
                },
            } as any);

            expect(get(voiceUsers).get(10)?.channel_id).toBe(0);
        });

        it('tracks muted and deafened state independently', () => {
            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 10, name: 'alice', display_name: null,
                    avatar_url: null, channel_id: 1,
                    self_mute: true, self_deaf: true, hash: null,
                },
            } as any);

            const user = get(voiceUsers).get(10);
            expect(user?.muted).toBe(true);
            expect(user?.deafened).toBe(true);
        });

        it('syncs local mute/deafen state for the local session', () => {
            handleMumbleEvent({ type: 'LocalSession', data: 10 } as any);
            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 10, name: 'me', display_name: null,
                    avatar_url: null, channel_id: 1,
                    self_mute: true, self_deaf: false, hash: null,
                },
            } as any);

            expect(get(isMuted)).toBe(true);
            expect(get(isDeafened)).toBe(false);
        });

        it('syncs both mute and deafen for the local session', () => {
            handleMumbleEvent({ type: 'LocalSession', data: 10 } as any);
            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 10, name: 'me', display_name: null,
                    avatar_url: null, channel_id: 1,
                    self_mute: true, self_deaf: true, hash: null,
                },
            } as any);

            expect(get(isMuted)).toBe(true);
            expect(get(isDeafened)).toBe(true);
        });

        it('does not sync mute/deafen for non-local users', () => {
            handleMumbleEvent({ type: 'LocalSession', data: 10 } as any);
            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 99, name: 'other', display_name: null,
                    avatar_url: null, channel_id: 1,
                    self_mute: true, self_deaf: true, hash: null,
                },
            } as any);

            expect(get(isMuted)).toBe(false);
            expect(get(isDeafened)).toBe(false);
        });

        it('does not sync local state when no LocalSession has been set', () => {
            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 10, name: 'me', display_name: null,
                    avatar_url: null, channel_id: 1,
                    self_mute: true, self_deaf: true, hash: null,
                },
            } as any);

            expect(get(isMuted)).toBe(false);
            expect(get(isDeafened)).toBe(false);
        });
    });

    describe('UserTalking', () => {
        it('adds a user to the talking set', () => {
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 10, talking: true } } as any);

            expect(get(talkingUsers).has(10)).toBe(true);
        });

        it('removes a user from the talking set', () => {
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 10, talking: true } } as any);
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 10, talking: false } } as any);

            expect(get(talkingUsers).has(10)).toBe(false);
        });
    });

    describe('UserVolume', () => {
        it('stores volume rounded to one decimal place', () => {
            handleMumbleEvent({ type: 'UserVolume', data: { session_id: 10, volume_db: -3.456 } } as any);

            expect(get(userVolumes)[10]).toBe(-3.5);
        });

        it('stores positive volume values', () => {
            handleMumbleEvent({ type: 'UserVolume', data: { session_id: 10, volume_db: 6.789 } } as any);

            expect(get(userVolumes)[10]).toBe(6.8);
        });

        it('stores zero volume', () => {
            handleMumbleEvent({ type: 'UserVolume', data: { session_id: 10, volume_db: 0 } } as any);

            expect(get(userVolumes)[10]).toBe(0);
        });

        it('tracks volumes for multiple users independently', () => {
            handleMumbleEvent({ type: 'UserVolume', data: { session_id: 10, volume_db: -3.0 } } as any);
            handleMumbleEvent({ type: 'UserVolume', data: { session_id: 20, volume_db: 5.0 } } as any);

            expect(get(userVolumes)[10]).toBe(-3.0);
            expect(get(userVolumes)[20]).toBe(5.0);
        });

        it('overwrites a previous volume for the same user', () => {
            handleMumbleEvent({ type: 'UserVolume', data: { session_id: 10, volume_db: -3.0 } } as any);
            handleMumbleEvent({ type: 'UserVolume', data: { session_id: 10, volume_db: 2.0 } } as any);

            expect(get(userVolumes)[10]).toBe(2.0);
        });
    });

    describe('UserRemoved', () => {
        it('removes a user from voiceUsers and talkingUsers', () => {
            addUser(10, 'alice', 1);
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 10, talking: true } } as any);

            handleMumbleEvent({ type: 'UserRemoved', data: 10 } as any);

            expect(get(voiceUsers).has(10)).toBe(false);
            expect(get(talkingUsers).has(10)).toBe(false);
        });

        it('does not throw when removing a nonexistent user', () => {
            handleMumbleEvent({ type: 'UserRemoved', data: 999 } as any);

            expect(get(voiceUsers).size).toBe(0);
        });

        it('only removes the targeted user, leaving others intact', () => {
            addUser(10, 'alice', 1);
            addUser(20, 'bob', 1);

            handleMumbleEvent({ type: 'UserRemoved', data: 10 } as any);

            expect(get(voiceUsers).has(10)).toBe(false);
            expect(get(voiceUsers).has(20)).toBe(true);
        });

        it('removes a user who was not talking without affecting talkingUsers', () => {
            addUser(10, 'alice', 1);
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 20, talking: true } } as any);

            handleMumbleEvent({ type: 'UserRemoved', data: 10 } as any);

            expect(get(talkingUsers).has(20)).toBe(true);
        });
    });

    describe('ConnectionState', () => {
        it('sets mumbleStatus to connected', () => {
            handleMumbleEvent({ type: 'ConnectionState', data: { type: 'Connected' } } as any);

            expect(get(mumbleStatus)).toBe('connected');
            expect(get(voiceConnected)).toBe(true);
        });

        it('sets mumbleStatus to connecting', () => {
            handleMumbleEvent({ type: 'ConnectionState', data: { type: 'Connecting' } } as any);

            expect(get(mumbleStatus)).toBe('connecting');
        });

        it('voiceConnected is false when connecting', () => {
            handleMumbleEvent({ type: 'ConnectionState', data: { type: 'Connecting' } } as any);

            expect(get(voiceConnected)).toBe(false);
        });

        it('voiceConnected is false when disconnected', () => {
            expect(get(voiceConnected)).toBe(false);
        });

        it('clears all voice state on disconnect', () => {
            handleMumbleEvent({ type: 'ChannelState', data: { id: 1, name: 'G', parent: 0 } } as any);
            addUser(10, 'alice', 1);
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 10, talking: true } } as any);

            handleMumbleEvent({ type: 'ConnectionState', data: { type: 'Disconnected' } } as any);

            expect(get(mumbleStatus)).toBe('disconnected');
            expect(get(voiceChannels).size).toBe(0);
            expect(get(voiceUsers).size).toBe(0);
            expect(get(talkingUsers).size).toBe(0);
        });

        it('transitions from connected to disconnected cleanly', () => {
            handleMumbleEvent({ type: 'ConnectionState', data: { type: 'Connected' } } as any);
            addUser(10, 'alice', 1);

            handleMumbleEvent({ type: 'ConnectionState', data: { type: 'Disconnected' } } as any);

            expect(get(mumbleStatus)).toBe('disconnected');
            expect(get(voiceConnected)).toBe(false);
            expect(get(voiceUsers).size).toBe(0);
        });

        it('disconnect is idempotent on already-disconnected state', () => {
            handleMumbleEvent({ type: 'ConnectionState', data: { type: 'Disconnected' } } as any);

            expect(get(mumbleStatus)).toBe('disconnected');
            expect(get(voiceChannels).size).toBe(0);
        });
    });

    describe('TransmissionModeChanged', () => {
        it('updates transmission mode when useMumbleSettings is true', async () => {
            const { useMumbleSettings, transmissionMode } = await import('../voiceSettings');
            useMumbleSettings.set(true);

            handleMumbleEvent({ type: 'TransmissionModeChanged', data: 'push_to_talk' } as any);

            expect(get(transmissionMode)).toBe('push_to_talk');
        });

        it('ignores transmission mode change when useMumbleSettings is false', async () => {
            const { useMumbleSettings, transmissionMode } = await import('../voiceSettings');
            useMumbleSettings.set(false);
            transmissionMode.set('voice_activation');

            handleMumbleEvent({ type: 'TransmissionModeChanged', data: 'push_to_talk' } as any);

            expect(get(transmissionMode)).toBe('voice_activation');
        });
    });

    describe('VadThresholdChanged', () => {
        it('updates vad threshold when useMumbleSettings is true', async () => {
            const { useMumbleSettings, vadThreshold } = await import('../voiceSettings');
            useMumbleSettings.set(true);

            handleMumbleEvent({ type: 'VadThresholdChanged', data: 0.75 } as any);

            expect(get(vadThreshold)).toBe(75);
        });

        it('ignores vad threshold change when useMumbleSettings is false', async () => {
            const { useMumbleSettings, vadThreshold } = await import('../voiceSettings');
            useMumbleSettings.set(false);
            vadThreshold.set(60);

            handleMumbleEvent({ type: 'VadThresholdChanged', data: 0.9 } as any);

            expect(get(vadThreshold)).toBe(60);
        });

        it('rounds to integer percentage', async () => {
            const { useMumbleSettings, vadThreshold } = await import('../voiceSettings');
            useMumbleSettings.set(true);

            handleMumbleEvent({ type: 'VadThresholdChanged', data: 0.333 } as any);

            expect(get(vadThreshold)).toBe(33);
        });
    });

    describe('VoiceHoldChanged', () => {
        it('updates voice hold when useMumbleSettings is true', async () => {
            const { useMumbleSettings, voiceHold } = await import('../voiceSettings');
            useMumbleSettings.set(true);

            handleMumbleEvent({ type: 'VoiceHoldChanged', data: 500 } as any);

            expect(get(voiceHold)).toBe(500);
        });

        it('ignores voice hold change when useMumbleSettings is false', async () => {
            const { useMumbleSettings, voiceHold } = await import('../voiceSettings');
            useMumbleSettings.set(false);
            voiceHold.set(250);

            handleMumbleEvent({ type: 'VoiceHoldChanged', data: 500 } as any);

            expect(get(voiceHold)).toBe(250);
        });
    });

    describe('LocalSession', () => {
        it('sets the local session so subsequent UserState syncs mute/deafen', () => {
            handleMumbleEvent({ type: 'LocalSession', data: 42 } as any);

            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 42, name: 'me', display_name: null,
                    avatar_url: null, channel_id: 1,
                    self_mute: true, self_deaf: false, hash: null,
                },
            } as any);

            expect(get(isMuted)).toBe(true);
        });

        it('is cleared on disconnect', () => {
            handleMumbleEvent({ type: 'LocalSession', data: 42 } as any);
            handleMumbleEvent({ type: 'ConnectionState', data: { type: 'Disconnected' } } as any);

            // After disconnect, session 42 should no longer be treated as local
            handleMumbleEvent({
                type: 'UserState',
                data: {
                    session_id: 42, name: 'me', display_name: null,
                    avatar_url: null, channel_id: 1,
                    self_mute: true, self_deaf: true, hash: null,
                },
            } as any);

            expect(get(isMuted)).toBe(false);
            expect(get(isDeafened)).toBe(false);
        });
    });

    describe('UserTalking (edge cases)', () => {
        it('setting talking to true twice does not duplicate', () => {
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 10, talking: true } } as any);
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 10, talking: true } } as any);

            expect(get(talkingUsers).size).toBe(1);
        });

        it('setting talking to false for a non-talking user is a no-op', () => {
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 10, talking: false } } as any);

            expect(get(talkingUsers).size).toBe(0);
        });

        it('tracks multiple users talking simultaneously', () => {
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 10, talking: true } } as any);
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 20, talking: true } } as any);
            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 30, talking: true } } as any);

            expect(get(talkingUsers).size).toBe(3);

            handleMumbleEvent({ type: 'UserTalking', data: { session_id: 20, talking: false } } as any);

            expect(get(talkingUsers).size).toBe(2);
            expect(get(talkingUsers).has(20)).toBe(false);
        });
    });
});

describe('handleSystemEvent', () => {
    it('updates display_name and avatar_url for matching username', () => {
        addUser(10, 'alice', 1);

        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'alice', display_name: 'Alice!', avatar_url: 'http://pic' },
        } as any);

        const user = get(voiceUsers).get(10);
        expect(user?.display_name).toBe('Alice!');
        expect(user?.avatar_url).toBe('http://pic');
    });

    it('updates all users with the same username', () => {
        // This is an edge case: multiple sessions with the same name
        addUser(10, 'alice', 1);
        addUser(20, 'alice', 2);

        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'alice', display_name: 'Alice Updated', avatar_url: 'http://new' },
        } as any);

        expect(get(voiceUsers).get(10)?.display_name).toBe('Alice Updated');
        expect(get(voiceUsers).get(20)?.display_name).toBe('Alice Updated');
    });

    it('ignores non-matching usernames', () => {
        addUser(10, 'alice', 1);

        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'bob', display_name: 'Bob', avatar_url: null },
        } as any);

        expect(get(voiceUsers).get(10)?.display_name).toBeNull();
    });

    it('ignores non-UserProfileChanged events', () => {
        addUser(10, 'alice', 1);

        handleSystemEvent({ type: 'ConnectionLost', data: {} } as any);

        expect(get(voiceUsers).get(10)?.name).toBe('alice');
    });

    it('does not create a new map reference when no users match', () => {
        addUser(10, 'alice', 1);
        const before = get(voiceUsers);

        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'nobody', display_name: 'X', avatar_url: null },
        } as any);

        // The optimization: unchanged maps should keep reference identity
        expect(get(voiceUsers)).toBe(before);
    });

    it('is a no-op when there are no voice users', () => {
        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'alice', display_name: 'Alice', avatar_url: null },
        } as any);

        expect(get(voiceUsers).size).toBe(0);
    });
});

describe('usersByChannel (derived)', () => {
    it('groups users by channel_id, sorted alphabetically by name', () => {
        addUser(1, 'zara', 5);
        addUser(2, 'alice', 5);
        addUser(3, 'bob', 7);

        const grouped = get(usersByChannel);
        expect(grouped.get(5)?.map(u => u.name)).toEqual(['alice', 'zara']);
        expect(grouped.get(7)?.map(u => u.name)).toEqual(['bob']);
    });

    it('returns empty map when no users exist', () => {
        expect(get(usersByChannel).size).toBe(0);
    });

    it('handles a single user in a channel', () => {
        addUser(1, 'alice', 5);

        const grouped = get(usersByChannel);
        expect(grouped.get(5)).toHaveLength(1);
        expect(grouped.get(5)?.[0].name).toBe('alice');
    });

    it('updates when a user changes channel', () => {
        addUser(1, 'alice', 5);
        addUser(2, 'bob', 5);

        // Move alice to channel 7
        handleMumbleEvent({
            type: 'UserState',
            data: {
                session_id: 1, name: null, display_name: null,
                avatar_url: null, channel_id: 7,
                self_mute: null, self_deaf: null, hash: null,
            },
        } as any);

        const grouped = get(usersByChannel);
        expect(grouped.get(5)?.map(u => u.name)).toEqual(['bob']);
        expect(grouped.get(7)?.map(u => u.name)).toEqual(['alice']);
    });

    it('removes channel group when its last user is removed', () => {
        addUser(1, 'alice', 5);

        handleMumbleEvent({ type: 'UserRemoved', data: 1 } as any);

        expect(get(usersByChannel).has(5)).toBe(false);
    });

    it('uses locale-aware alphabetical sorting', () => {
        addUser(1, 'Charlie', 5);
        addUser(2, 'alice', 5);
        addUser(3, 'Bob', 5);

        const names = get(usersByChannel).get(5)?.map(u => u.name);
        // localeCompare sorts case-insensitively by default in most locales
        expect(names).toEqual(['alice', 'Bob', 'Charlie']);
    });
});
