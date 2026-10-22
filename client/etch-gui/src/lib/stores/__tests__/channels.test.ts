import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { channels, activeChannel, handleMatrixEvent, initHiddenDms, hideDm, unhideDm } from '../channels';
import { activeChannelId } from '../activeChannel';
import { handleMatrixEvent as newMessageHandleMatrixEvent } from '../newMessage';
import { setDrafting, setReply, setEditing } from '../compose';
import { followNewMessages } from '../followMessages';
import { resetStores } from './helpers';
import type { RoomInfo } from '$lib/types';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

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

    describe('a new message (hidden DM auto-unhide)', () => {
        it('unhides a hidden DM when the other person messages', () => {
            initHiddenDms(['dm1']);
            handleMatrixEvent({
                type: 'ChannelList',
                data: [makeRoom('t1', 'General'), makeRoom('dm1', 'Alice', 'Dm')],
            } as any);
            expect(get(channels).find(c => c.id === 'dm1')).toBeUndefined();

            receive('dm1');

            expect(get(channels).find(c => c.id === 'dm1')).toBeDefined();
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

describe('hideDm / unhideDm (store-level)', () => {
    it('unhideDm is a no-op for a non-hidden DM', () => {
        const rooms = [makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')];
        handleMatrixEvent({ type: 'ChannelList', data: rooms } as any);

        unhideDm('dm1');

        // Channel should still be there, unaffected
        expect(get(channels).find(c => c.id === 'dm1')).toBeDefined();
    });

    it('unhideDm restores the channel', () => {
        const rooms = [makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')];
        channels.set(rooms);
        activeChannelId.set('t1');

        hideDm('dm1');
        expect(get(channels).find(c => c.id === 'dm1')).toBeUndefined();

        unhideDm('dm1');
        expect(get(channels).find(c => c.id === 'dm1')).toBeDefined();
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

function receive(roomId: string) {
    newMessageHandleMatrixEvent({ type: 'NewMessage', data: { room_id: roomId, sender_name: 'Alice', body: 'hi' } });
}

describe('following new messages', () => {
    beforeEach(() => {
        handleMatrixEvent({ type: 'ChannelList', data: [makeRoom('t1', 'General'), makeRoom('dm1', 'Alice', 'Dm')] } as any);
        activeChannelId.set('t1');
    });

    it('leaves the active channel alone when off', () => {
        receive('dm1');

        expect(get(activeChannelId)).toBe('t1');
    });

    it('switches to the room a message arrived in', () => {
        followNewMessages.set(true);

        receive('dm1');

        expect(get(activeChannelId)).toBe('dm1');
    });

    it.each([
        ['a draft', () => setDrafting(true)],
        ['a reply', () => setReply({ id: 'e0' } as any)],
        ['an edit', () => setEditing({ id: 'e0' } as any)],
    ])('stays put while %s is in the composer', (_, compose) => {
        followNewMessages.set(true);
        compose();

        receive('dm1');

        expect(get(activeChannelId)).toBe('t1');
    });

    it('switches to a hidden DM that a message brings back', () => {
        followNewMessages.set(true);
        hideDm('dm1');

        receive('dm1');

        expect(get(activeChannelId)).toBe('dm1');
    });
});
