import { describe, it, expect, beforeEach, beforeAll, vi } from 'vitest';
import { get } from 'svelte/store';
import { channels, activeChannel, dmLastActivity, handleMatrixEvent, initChannels, initHiddenDms, hideDm, unhideDm, resetChannels } from '../channels';
import { activeChannelId } from '../activeChannel';
import { currentUser } from '../user';
import { resetStores } from './helpers';
import type { RoomInfo } from '$lib/types';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

// initChannels creates subscriptions that stack -- call once for the whole suite
beforeAll(() => {
    initChannels();
});

beforeEach(() => {
    resetStores();
});

function makeRoom(id: string, name: string, type: 'Voice' | 'Text' | 'Dm' = 'Text'): RoomInfo {
    return {
        id,
        display_name: name,
        etch_room_type: type,
        channel_id: null,
        is_default: false,
        unread_count: 0,
        is_encrypted: false,
        avatar_url: null,
    };
}

describe('handleMatrixEvent (channels)', () => {
    describe('ChannelList', () => {
        it('sets the channel list', () => {
            const rooms = [makeRoom('t1', 'General'), makeRoom('t2', 'Random')];
            handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

            expect(get(channels)).toHaveLength(2);
        });

        it('auto-selects the first voice channel', () => {
            const rooms = [makeRoom('t1', 'General', 'Text'), makeRoom('v1', 'Lounge', 'Voice')];
            handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

            expect(get(activeChannelId)).toBe('v1');
        });

        it('falls back to the first channel if no voice channel exists', () => {
            const rooms = [makeRoom('t1', 'Chat'), makeRoom('t2', 'Logs')];
            handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

            expect(get(activeChannelId)).toBe('t1');
        });

        it('replaces the previous channel list entirely', () => {
            handleMatrixEvent({ type: 'ChannelList', data: [makeRoom('t1', 'Old'), makeRoom('t2', 'AlsoOld')] } as any);
            handleMatrixEvent({ type: 'ChannelList', data: [makeRoom('t3', 'New')] } as any);

            expect(get(channels)).toHaveLength(1);
            expect(get(channels)[0].id).toBe('t3');
        });

        it('handles an empty channel list without crashing', () => {
            handleMatrixEvent({ type: 'ChannelList', data: [] } as any);

            expect(get(channels)).toHaveLength(0);
        });

        it('does not select a channel when the list is empty', () => {
            handleMatrixEvent({ type: 'ChannelList', data: [] } as any);

            // activeChannelId stays null because there's nothing to select
            expect(get(activeChannel)).toBeNull();
        });

        it('filters out hidden DMs', () => {
            initHiddenDms(['dm1']);
            const rooms = [makeRoom('t1', 'General'), makeRoom('dm1', 'Alice', 'Dm')];
            handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

            expect(get(channels).find(c => c.id === 'dm1')).toBeUndefined();
            expect(get(channels)).toHaveLength(1);
        });

        it('selects from visible channels when all DMs are hidden', () => {
            initHiddenDms(['dm1', 'dm2']);
            const rooms = [makeRoom('dm1', 'Alice', 'Dm'), makeRoom('dm2', 'Bob', 'Dm'), makeRoom('t1', 'General')];
            handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

            expect(get(channels)).toHaveLength(1);
            expect(get(activeChannelId)).toBe('t1');
        });

        it('prefers Voice over Text even when Text appears first', () => {
            const rooms = [
                makeRoom('t1', 'Text1', 'Text'),
                makeRoom('t2', 'Text2', 'Text'),
                makeRoom('v1', 'Voice1', 'Voice'),
            ];
            handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

            expect(get(activeChannelId)).toBe('v1');
        });
    });

    describe('DmCreated', () => {
        it('adds a new DM and selects it', () => {
            handleMatrixEvent({ type: 'ChannelList', data: [makeRoom('t1', 'General')] } as any);

            const dm = makeRoom('dm1', 'Bob', 'Dm');
            handleMatrixEvent({ type: 'DmCreated', data: dm } as any);

            expect(get(channels).find(c => c.id === 'dm1')).toBeDefined();
            expect(get(activeChannelId)).toBe('dm1');
        });

        it('does not duplicate an existing channel', () => {
            const dm = makeRoom('dm1', 'Bob', 'Dm');
            handleMatrixEvent({ type: 'ChannelList', data: [dm] } as any);
            handleMatrixEvent({ type: 'DmCreated', data: dm } as any);

            expect(get(channels).filter(c => c.id === 'dm1')).toHaveLength(1);
        });

        it('still selects the channel even when it already exists', () => {
            const dm = makeRoom('dm1', 'Bob', 'Dm');
            handleMatrixEvent({ type: 'ChannelList', data: [makeRoom('t1', 'General'), dm] } as any);
            activeChannelId.set('t1');

            handleMatrixEvent({ type: 'DmCreated', data: dm } as any);

            expect(get(activeChannelId)).toBe('dm1');
        });

        it('appends the DM to the end of the channel list', () => {
            handleMatrixEvent({ type: 'ChannelList', data: [makeRoom('t1', 'General'), makeRoom('t2', 'Random')] } as any);

            handleMatrixEvent({ type: 'DmCreated', data: makeRoom('dm1', 'Bob', 'Dm') } as any);

            const list = get(channels);
            expect(list[list.length - 1].id).toBe('dm1');
        });
    });

    describe('TimelinePushBack (hidden DM auto-unhide)', () => {
        it('unhides a hidden DM when the other person messages', () => {
            currentUser.set({ username: 'me', matrixId: '@me:s', displayName: null, avatarUrl: null });
            initHiddenDms(['dm1']);

            // Set up channels so the hidden DM info is stashed
            handleMatrixEvent({
                type: 'ChannelList',
                data: [makeRoom('t1', 'General'), makeRoom('dm1', 'Alice', 'Dm')],
            } as any);
            expect(get(channels).find(c => c.id === 'dm1')).toBeUndefined();

            // Simulate incoming message from the other person
            handleMatrixEvent({
                type: 'TimelinePushBack',
                data: ['dm1', {
                    sender: { display_name: 'Alice', avatar_url: null },
                    kind: { Message: { id: 'e1', sender: '@alice:s', body: 'hey', html_body: null, media: null, timestamp: Date.now(), reactions: {} } },
                }],
            } as any);

            expect(get(channels).find(c => c.id === 'dm1')).toBeDefined();
        });

        it('does not unhide when the message is from self', () => {
            currentUser.set({ username: 'me', matrixId: '@me:s', displayName: null, avatarUrl: null });
            initHiddenDms(['dm1']);

            handleMatrixEvent({
                type: 'ChannelList',
                data: [makeRoom('t1', 'General'), makeRoom('dm1', 'Alice', 'Dm')],
            } as any);

            handleMatrixEvent({
                type: 'TimelinePushBack',
                data: ['dm1', {
                    sender: { display_name: 'Me', avatar_url: null },
                    kind: { Message: { id: 'e1', sender: '@me:s', body: 'test', html_body: null, media: null, timestamp: Date.now(), reactions: {} } },
                }],
            } as any);

            expect(get(channels).find(c => c.id === 'dm1')).toBeUndefined();
        });
    });
});

describe('activeChannel (derived)', () => {
    it('returns the channel matching activeChannelId', () => {
        handleMatrixEvent({
            type: 'ChannelList',
            data: [makeRoom('t1', 'General'), makeRoom('t2', 'Random')],
        } as any);
        activeChannelId.set('t2');

        expect(get(activeChannel)?.display_name).toBe('Random');
    });

    it('returns null when no channel matches', () => {
        activeChannelId.set('nonexistent');

        expect(get(activeChannel)).toBeNull();
    });

    it('returns null when activeChannelId is null', () => {
        handleMatrixEvent({ type: 'ChannelList', data: [makeRoom('t1', 'General')] } as any);
        activeChannelId.set(null);

        expect(get(activeChannel)).toBeNull();
    });

    it('returns null when channel list is empty', () => {
        handleMatrixEvent({ type: 'ChannelList', data: [] } as any);
        activeChannelId.set('t1');

        expect(get(activeChannel)).toBeNull();
    });

    it('updates reactively when channel list changes', () => {
        handleMatrixEvent({ type: 'ChannelList', data: [makeRoom('t1', 'General')] } as any);
        activeChannelId.set('t1');
        expect(get(activeChannel)?.display_name).toBe('General');

        // Replace channel list without the active channel
        handleMatrixEvent({ type: 'ChannelList', data: [makeRoom('t2', 'Random')] } as any);

        // t1 no longer exists, so the auto-selection logic will change activeChannelId
        // but if we manually check, the derived store should reflect the new state
        expect(get(activeChannel)).not.toBeNull();
    });
});

describe('unread count tracking', () => {
    it('clears unread count when switching to a channel', () => {
        const rooms = [
            { ...makeRoom('t1', 'General'), unread_count: 5 },
            makeRoom('t2', 'Random'),
        ];
        handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

        activeChannelId.set('t1');

        expect(get(channels).find(c => c.id === 't1')?.unread_count).toBe(0);
    });

    it('does not affect other channels when clearing unread', () => {
        const rooms = [
            { ...makeRoom('t1', 'General'), unread_count: 5 },
            { ...makeRoom('t2', 'Random'), unread_count: 3 },
        ];
        handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

        activeChannelId.set('t1');

        expect(get(channels).find(c => c.id === 't2')?.unread_count).toBe(3);
    });

    it('clears unread to 0 regardless of the count', () => {
        const rooms = [
            { ...makeRoom('t1', 'General'), unread_count: 999 },
        ];
        handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

        activeChannelId.set('t1');

        expect(get(channels).find(c => c.id === 't1')?.unread_count).toBe(0);
    });
});

describe('hideDm / unhideDm (store-level)', () => {
    it('unhideDm is a no-op for a non-hidden DM', () => {
        const rooms = [makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')];
        handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

        unhideDm('dm1');

        // Channel should still be there, unaffected
        expect(get(channels).find(c => c.id === 'dm1')).toBeDefined();
    });

    it('unhideDm restores the channel with unread_count of 1', () => {
        const rooms = [makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')];
        channels.set(rooms);
        activeChannelId.set('t1');

        hideDm('dm1');
        expect(get(channels).find(c => c.id === 'dm1')).toBeUndefined();

        unhideDm('dm1');
        const restored = get(channels).find(c => c.id === 'dm1');
        expect(restored).toBeDefined();
        expect(restored?.unread_count).toBe(1);
    });

    it('hideDm switches active channel when hiding the active DM', () => {
        const rooms = [
            makeRoom('dm1', 'Alice', 'Dm'),
            makeRoom('v1', 'Voice', 'Voice'),
        ];
        channels.set(rooms);
        activeChannelId.set('dm1');

        hideDm('dm1');

        expect(get(activeChannelId)).not.toBe('dm1');
    });

    it('hideDm does not switch active channel when hiding a non-active DM', () => {
        const rooms = [
            makeRoom('dm1', 'Alice', 'Dm'),
            makeRoom('t1', 'General'),
        ];
        channels.set(rooms);
        activeChannelId.set('t1');

        hideDm('dm1');

        expect(get(activeChannelId)).toBe('t1');
    });
});

// --- Helpers for dmLastActivity tests ---

function makeMessage(id: string, sender: string, timestamp: number) {
    return {
        sender: { display_name: sender, avatar_url: null },
        kind: { Message: { id, sender: `@${sender}:s`, body: 'hi', html_body: null, media: null, timestamp, reactions: {} } },
    };
}

function makeStateEvent() {
    return {
        sender: { display_name: 'system', avatar_url: null },
        kind: { StateEvent: { RoomNameChanged: { name: 'New Name' } } },
    };
}

function makeDayDivider(ts: number) {
    return { sender: null, kind: { DayDivider: ts } };
}

describe('dmLastActivity', () => {
    describe('TimelinePushBack', () => {
        it('updates timestamp when a message arrives', () => {
            handleMatrixEvent({
                type: 'TimelinePushBack',
                data: ['dm1', makeMessage('e1', 'alice', 1000)],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBe(1000);
        });

        it('overwrites with a newer timestamp', () => {
            handleMatrixEvent({
                type: 'TimelinePushBack',
                data: ['dm1', makeMessage('e1', 'alice', 1000)],
            } as any);
            handleMatrixEvent({
                type: 'TimelinePushBack',
                data: ['dm1', makeMessage('e2', 'alice', 2000)],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBe(2000);
        });

        it('does not update for non-message entries (state events)', () => {
            handleMatrixEvent({
                type: 'TimelinePushBack',
                data: ['dm1', makeStateEvent()],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBeUndefined();
        });

        it('does not update for string-kind entries (ReadMarker)', () => {
            handleMatrixEvent({
                type: 'TimelinePushBack',
                data: ['dm1', { sender: null, kind: 'ReadMarker' }],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBeUndefined();
        });

        it('tracks multiple rooms independently', () => {
            handleMatrixEvent({
                type: 'TimelinePushBack',
                data: ['dm1', makeMessage('e1', 'alice', 1000)],
            } as any);
            handleMatrixEvent({
                type: 'TimelinePushBack',
                data: ['dm2', makeMessage('e2', 'bob', 2000)],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBe(1000);
            expect(get(dmLastActivity)['dm2']).toBe(2000);
        });
    });

    describe('TimelineAppend', () => {
        it('picks the timestamp of the last message in the batch', () => {
            handleMatrixEvent({
                type: 'TimelineAppend',
                data: ['dm1', [
                    makeMessage('e1', 'alice', 1000),
                    makeMessage('e2', 'alice', 2000),
                    makeMessage('e3', 'bob', 3000),
                ]],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBe(3000);
        });

        it('skips non-message entries at the end of the batch', () => {
            handleMatrixEvent({
                type: 'TimelineAppend',
                data: ['dm1', [
                    makeMessage('e1', 'alice', 5000),
                    makeDayDivider(86400000),
                ]],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBe(5000);
        });

        it('does not update when batch contains no messages', () => {
            handleMatrixEvent({
                type: 'TimelineAppend',
                data: ['dm1', [makeDayDivider(86400000), makeStateEvent()]],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBeUndefined();
        });

        it('does not overwrite a newer timestamp with older history', () => {
            // Simulate: a new message arrives first, then initial history loads
            handleMatrixEvent({
                type: 'TimelinePushBack',
                data: ['dm1', makeMessage('e-new', 'alice', 9000)],
            } as any);
            handleMatrixEvent({
                type: 'TimelineAppend',
                data: ['dm1', [
                    makeMessage('e-old1', 'bob', 1000),
                    makeMessage('e-old2', 'bob', 2000),
                ]],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBe(9000);
        });

        it('updates when the appended timestamp equals the existing one', () => {
            dmLastActivity.set({ dm1: 5000 });

            handleMatrixEvent({
                type: 'TimelineAppend',
                data: ['dm1', [makeMessage('e1', 'alice', 5000)]],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBe(5000);
        });
    });

    describe('TimelineReset', () => {
        it('replaces the timestamp with the last message in the new timeline', () => {
            dmLastActivity.set({ dm1: 1000 });

            handleMatrixEvent({
                type: 'TimelineReset',
                data: ['dm1', [
                    makeMessage('e1', 'alice', 5000),
                    makeMessage('e2', 'bob', 8000),
                ]],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBe(8000);
        });

        it('preserves existing timestamp when reset timeline has no messages', () => {
            dmLastActivity.set({ dm1: 5000 });

            handleMatrixEvent({
                type: 'TimelineReset',
                data: ['dm1', [makeDayDivider(86400000)]],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBe(5000);
        });

        it('can set a timestamp for a room that had none', () => {
            handleMatrixEvent({
                type: 'TimelineReset',
                data: ['dm1', [makeMessage('e1', 'alice', 4000)]],
            } as any);

            expect(get(dmLastActivity)['dm1']).toBe(4000);
        });
    });

    describe('resetChannels', () => {
        it('clears dmLastActivity', () => {
            dmLastActivity.set({ dm1: 1000, dm2: 2000 });

            resetChannels();

            expect(get(dmLastActivity)).toEqual({});
        });
    });
});
