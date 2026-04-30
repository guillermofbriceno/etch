import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { activeWindow, handleMatrixEvent } from '../messages';
import { activeChannelId } from '../activeChannel';
import { currentUser } from '../user';
import { appFocused } from '../eventRouter';
import { resetStores } from './helpers';
import type { TimelineEntry } from '$lib/types';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

import * as sfx from '../sfx';

beforeEach(() => {
    resetStores();
    vi.mocked(sfx.playSfx).mockClear();
});

function makeEntry(id: string, body: string): TimelineEntry {
    return {
        sender: { display_name: 'Test', avatar_url: null },
        kind: {
            Message: {
                id,
                sender: '@test:s',
                body,
                html_body: null,
                media: null,
                timestamp: Date.now(),
                reactions: {},
            },
        },
    };
}

function getBody(entry: TimelineEntry): string | null {
    const kind = entry.kind;
    if (typeof kind === 'object' && 'Message' in kind) return kind.Message.body;
    return null;
}

describe('handleMatrixEvent (timeline)', () => {
    const ROOM = 'room1';

    beforeEach(() => {
        activeChannelId.set(ROOM);
        // Clear the private windows store for this room (it persists across tests)
        handleMatrixEvent({ type: 'TimelineCleared', data: ROOM } as any);
    });

    describe('TimelineAppend', () => {
        it('appends multiple entries to a room', () => {
            const entries = [makeEntry('e1', 'hi'), makeEntry('e2', 'hey')];
            handleMatrixEvent({ type: 'TimelineAppend', data: [ROOM, entries] } as any);

            const win = get(activeWindow);
            expect(win.entries).toHaveLength(2);
            expect(getBody(win.entries[0])).toBe('hi');
            expect(getBody(win.entries[1])).toBe('hey');
        });

        it('appends to existing entries (accumulation)', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e0', 'existing')] } as any);
            handleMatrixEvent({ type: 'TimelineAppend', data: [ROOM, [makeEntry('e1', 'new1'), makeEntry('e2', 'new2')]] } as any);

            const entries = get(activeWindow).entries;
            expect(entries).toHaveLength(3);
            expect(getBody(entries[0])).toBe('existing');
            expect(getBody(entries[1])).toBe('new1');
        });

        it('handles empty array append without changing state', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'hi')] } as any);
            handleMatrixEvent({ type: 'TimelineAppend', data: [ROOM, []] } as any);

            expect(get(activeWindow).entries).toHaveLength(1);
        });
    });

    describe('TimelinePushBack', () => {
        it('appends a single entry to the end', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'hello')] } as any);

            expect(get(activeWindow).entries).toHaveLength(1);
            expect(getBody(get(activeWindow).entries[0])).toBe('hello');
        });

        it('builds up entries in order', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'first')] } as any);
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e2', 'second')] } as any);

            const entries = get(activeWindow).entries;
            expect(entries).toHaveLength(2);
            expect(getBody(entries[0])).toBe('first');
            expect(getBody(entries[1])).toBe('second');
        });
    });

    describe('TimelinePushFront', () => {
        it('prepends an entry to the beginning', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e2', 'second')] } as any);
            handleMatrixEvent({ type: 'TimelinePushFront', data: [ROOM, makeEntry('e1', 'first')] } as any);

            const entries = get(activeWindow).entries;
            expect(entries).toHaveLength(2);
            expect(getBody(entries[0])).toBe('first');
            expect(getBody(entries[1])).toBe('second');
        });

        it('multiple prepends maintain reverse insertion order', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e3', 'third')] } as any);
            handleMatrixEvent({ type: 'TimelinePushFront', data: [ROOM, makeEntry('e2', 'second')] } as any);
            handleMatrixEvent({ type: 'TimelinePushFront', data: [ROOM, makeEntry('e1', 'first')] } as any);

            const entries = get(activeWindow).entries;
            expect(entries).toHaveLength(3);
            expect(getBody(entries[0])).toBe('first');
            expect(getBody(entries[1])).toBe('second');
            expect(getBody(entries[2])).toBe('third');
        });

        it('prepends to an empty room', () => {
            handleMatrixEvent({ type: 'TimelinePushFront', data: [ROOM, makeEntry('e1', 'solo')] } as any);

            expect(get(activeWindow).entries).toHaveLength(1);
            expect(getBody(get(activeWindow).entries[0])).toBe('solo');
        });
    });

    describe('TimelineInsert', () => {
        it('inserts an entry at a specific index', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'first')] } as any);
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e3', 'third')] } as any);
            handleMatrixEvent({ type: 'TimelineInsert', data: [ROOM, 1, makeEntry('e2', 'middle')] } as any);

            const entries = get(activeWindow).entries;
            expect(entries).toHaveLength(3);
            expect(getBody(entries[1])).toBe('middle');
        });

        it('inserts at index 0 (beginning)', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e2', 'second')] } as any);
            handleMatrixEvent({ type: 'TimelineInsert', data: [ROOM, 0, makeEntry('e1', 'first')] } as any);

            expect(getBody(get(activeWindow).entries[0])).toBe('first');
            expect(getBody(get(activeWindow).entries[1])).toBe('second');
        });

        it('inserts at the end (index === length)', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'first')] } as any);
            handleMatrixEvent({ type: 'TimelineInsert', data: [ROOM, 1, makeEntry('e2', 'second')] } as any);

            const entries = get(activeWindow).entries;
            expect(entries).toHaveLength(2);
            expect(getBody(entries[1])).toBe('second');
        });

        it('no-ops on a nonexistent room', () => {
            handleMatrixEvent({ type: 'TimelineInsert', data: ['nonexistent', 0, makeEntry('e1', 'x')] } as any);
            // Should not crash; nonexistent room has no window yet
        });
    });

    describe('TimelineSet', () => {
        it('replaces an entry at a specific index', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'original')] } as any);
            handleMatrixEvent({ type: 'TimelineSet', data: [ROOM, 0, makeEntry('e1', 'edited')] } as any);

            expect(getBody(get(activeWindow).entries[0])).toBe('edited');
        });

        it('ignores out-of-bounds index', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'only')] } as any);
            handleMatrixEvent({ type: 'TimelineSet', data: [ROOM, 5, makeEntry('e2', 'nope')] } as any);

            expect(get(activeWindow).entries).toHaveLength(1);
            expect(getBody(get(activeWindow).entries[0])).toBe('only');
        });

        it('ignores negative index', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'only')] } as any);
            handleMatrixEvent({ type: 'TimelineSet', data: [ROOM, -1, makeEntry('e2', 'nope')] } as any);

            expect(getBody(get(activeWindow).entries[0])).toBe('only');
        });

        it('no-ops on a nonexistent room', () => {
            handleMatrixEvent({ type: 'TimelineSet', data: ['nonexistent', 0, makeEntry('e1', 'x')] } as any);
            // Should not crash
        });

        it('replaces the last entry in a multi-entry list', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'first')] } as any);
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e2', 'second')] } as any);
            handleMatrixEvent({ type: 'TimelineSet', data: [ROOM, 1, makeEntry('e2', 'edited-second')] } as any);

            const entries = get(activeWindow).entries;
            expect(getBody(entries[0])).toBe('first');
            expect(getBody(entries[1])).toBe('edited-second');
        });
    });

    describe('TimelineRemove', () => {
        it('removes an entry at a specific index', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'a')] } as any);
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e2', 'b')] } as any);
            handleMatrixEvent({ type: 'TimelineRemove', data: [ROOM, 0] } as any);

            const entries = get(activeWindow).entries;
            expect(entries).toHaveLength(1);
            expect(getBody(entries[0])).toBe('b');
        });

        it('removes the last entry leaving an empty list', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'only')] } as any);
            handleMatrixEvent({ type: 'TimelineRemove', data: [ROOM, 0] } as any);

            expect(get(activeWindow).entries).toHaveLength(0);
        });

        it('ignores out-of-bounds index', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'only')] } as any);
            handleMatrixEvent({ type: 'TimelineRemove', data: [ROOM, 5] } as any);

            expect(get(activeWindow).entries).toHaveLength(1);
        });

        it('ignores negative index', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'only')] } as any);
            handleMatrixEvent({ type: 'TimelineRemove', data: [ROOM, -1] } as any);

            expect(get(activeWindow).entries).toHaveLength(1);
        });

        it('no-ops on a nonexistent room', () => {
            handleMatrixEvent({ type: 'TimelineRemove', data: ['nonexistent', 0] } as any);
            // Should not crash
        });
    });

    describe('TimelineCleared', () => {
        it('resets a room to empty with hasMore true', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'hi')] } as any);
            handleMatrixEvent({ type: 'TimelineCleared', data: ROOM } as any);

            const win = get(activeWindow);
            expect(win.entries).toHaveLength(0);
            expect(win.hasMore).toBe(true);
        });
    });

    describe('TimelineReset', () => {
        it('replaces all entries in a room', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'old')] } as any);
            handleMatrixEvent({ type: 'TimelineReset', data: [ROOM, [makeEntry('e2', 'fresh')]] } as any);

            const win = get(activeWindow);
            expect(win.entries).toHaveLength(1);
            expect(getBody(win.entries[0])).toBe('fresh');
            expect(win.hasMore).toBe(true);
        });

        it('resets to empty array', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'old')] } as any);
            handleMatrixEvent({ type: 'TimelineReset', data: [ROOM, []] } as any);

            const win = get(activeWindow);
            expect(win.entries).toHaveLength(0);
            expect(win.hasMore).toBe(true);
        });

        it('resets loading state to false', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'x')] } as any);
            // Simulate a loading state
            handleMatrixEvent({ type: 'TimelineReset', data: [ROOM, [makeEntry('e2', 'y')]] } as any);

            expect(get(activeWindow).loading).toBe(false);
        });
    });

    describe('PaginationComplete', () => {
        it('clears loading and sets hasMore', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'hi')] } as any);
            handleMatrixEvent({ type: 'PaginationComplete', data: [ROOM, false] } as any);

            const win = get(activeWindow);
            expect(win.loading).toBe(false);
            expect(win.hasMore).toBe(false);
        });

        it('preserves hasMore when there is more history', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'hi')] } as any);
            handleMatrixEvent({ type: 'PaginationComplete', data: [ROOM, true] } as any);

            expect(get(activeWindow).hasMore).toBe(true);
        });

        it('no-ops on a nonexistent room', () => {
            handleMatrixEvent({ type: 'PaginationComplete', data: ['nonexistent', false] } as any);
            // Should not crash
        });

        it('does not affect entries', () => {
            handleMatrixEvent({ type: 'TimelinePushBack', data: [ROOM, makeEntry('e1', 'hi')] } as any);
            handleMatrixEvent({ type: 'PaginationComplete', data: [ROOM, false] } as any);

            expect(get(activeWindow).entries).toHaveLength(1);
            expect(getBody(get(activeWindow).entries[0])).toBe('hi');
        });
    });
});

describe('cross-room isolation', () => {
    it('messages in one room do not appear in another', () => {
        activeChannelId.set('room-a');
        handleMatrixEvent({ type: 'TimelineCleared', data: 'room-a' } as any);
        handleMatrixEvent({ type: 'TimelineCleared', data: 'room-b' } as any);

        handleMatrixEvent({ type: 'TimelinePushBack', data: ['room-a', makeEntry('e1', 'in A')] } as any);
        handleMatrixEvent({ type: 'TimelinePushBack', data: ['room-b', makeEntry('e2', 'in B')] } as any);

        activeChannelId.set('room-a');
        expect(get(activeWindow).entries).toHaveLength(1);
        expect(getBody(get(activeWindow).entries[0])).toBe('in A');

        activeChannelId.set('room-b');
        expect(get(activeWindow).entries).toHaveLength(1);
        expect(getBody(get(activeWindow).entries[0])).toBe('in B');
    });

    it('clearing one room does not affect another', () => {
        activeChannelId.set('room-a');
        handleMatrixEvent({ type: 'TimelineCleared', data: 'room-a' } as any);
        handleMatrixEvent({ type: 'TimelineCleared', data: 'room-b' } as any);

        handleMatrixEvent({ type: 'TimelinePushBack', data: ['room-a', makeEntry('e1', 'A msg')] } as any);
        handleMatrixEvent({ type: 'TimelinePushBack', data: ['room-b', makeEntry('e2', 'B msg')] } as any);

        handleMatrixEvent({ type: 'TimelineCleared', data: 'room-a' } as any);

        activeChannelId.set('room-a');
        expect(get(activeWindow).entries).toHaveLength(0);

        activeChannelId.set('room-b');
        expect(get(activeWindow).entries).toHaveLength(1);
    });

    it('removing an entry in one room does not shift indices in another', () => {
        activeChannelId.set('room-a');
        handleMatrixEvent({ type: 'TimelineCleared', data: 'room-a' } as any);
        handleMatrixEvent({ type: 'TimelineCleared', data: 'room-b' } as any);

        handleMatrixEvent({ type: 'TimelinePushBack', data: ['room-a', makeEntry('a1', 'A1')] } as any);
        handleMatrixEvent({ type: 'TimelinePushBack', data: ['room-a', makeEntry('a2', 'A2')] } as any);
        handleMatrixEvent({ type: 'TimelinePushBack', data: ['room-b', makeEntry('b1', 'B1')] } as any);

        handleMatrixEvent({ type: 'TimelineRemove', data: ['room-a', 0] } as any);

        activeChannelId.set('room-b');
        expect(get(activeWindow).entries).toHaveLength(1);
        expect(getBody(get(activeWindow).entries[0])).toBe('B1');
    });
});

describe('activeWindow (derived)', () => {
    it('returns EMPTY_WINDOW when no channel is selected', () => {
        activeChannelId.set(null);
        const win = get(activeWindow);
        expect(win.entries).toHaveLength(0);
        expect(win.hasMore).toBe(true);
        expect(win.loading).toBe(false);
    });

    it('returns EMPTY_WINDOW for a channel with no messages', () => {
        activeChannelId.set('empty-room');
        const win = get(activeWindow);
        expect(win.entries).toHaveLength(0);
    });

    it('switches when activeChannelId changes', () => {
        // Clear rooms first -- windows store is private and not reset by resetStores()
        handleMatrixEvent({ type: 'TimelineCleared', data: 'room-a' } as any);
        handleMatrixEvent({ type: 'TimelineCleared', data: 'room-b' } as any);

        activeChannelId.set('room-a');
        handleMatrixEvent({ type: 'TimelinePushBack', data: ['room-a', makeEntry('e1', 'in A')] } as any);

        activeChannelId.set('room-b');
        handleMatrixEvent({ type: 'TimelinePushBack', data: ['room-b', makeEntry('e2', 'in B')] } as any);

        expect(getBody(get(activeWindow).entries[0])).toBe('in B');

        activeChannelId.set('room-a');
        expect(getBody(get(activeWindow).entries[0])).toBe('in A');
    });

    it('EMPTY_WINDOW has correct default values', () => {
        activeChannelId.set(null);
        const win = get(activeWindow);

        expect(win.entries).toEqual([]);
        expect(win.hasMore).toBe(true);
        expect(win.loading).toBe(false);
    });
});

describe('notification sounds', () => {
    const ROOM = 'notif-room';

    // lastNotifTime is a private module-level variable that persists between
    // tests. Each test needs a fake clock that's at least 20s ahead of the
    // previous test's notification time. We increment the base by 30s per test
    // so the cooldown always passes at the start of each test.
    let fakeTime = Date.UTC(2099, 0, 1);

    function pushMessage(roomId: string, senderId: string, body: string) {
        handleMatrixEvent({
            type: 'TimelinePushBack',
            data: [roomId, {
                sender: { display_name: 'User', avatar_url: null },
                kind: { Message: { id: `$${body}`, sender: senderId, body, html_body: null, media: null, timestamp: Date.now(), reactions: {} } },
            }],
        } as any);
    }

    beforeEach(() => {
        fakeTime += 30_000;
        vi.useFakeTimers({ now: fakeTime });
        handleMatrixEvent({ type: 'TimelineCleared', data: ROOM } as any);
        currentUser.set({ username: 'me', matrixId: '@me:s', displayName: null, avatarUrl: null });
        activeChannelId.set(ROOM);
        appFocused.set(true);
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it('does not play sound for self-messages', () => {
        pushMessage(ROOM, '@me:s', 'my own message');

        expect(vi.mocked(sfx.playSfx)).not.toHaveBeenCalledWith('new_notif');
    });

    it('does not play sound when app is focused and message is in the active room', () => {
        appFocused.set(true);
        activeChannelId.set(ROOM);

        pushMessage(ROOM, '@other:s', 'hello');

        expect(vi.mocked(sfx.playSfx)).not.toHaveBeenCalledWith('new_notif');
    });

    it('plays sound when message is in a non-active room', () => {
        appFocused.set(true);
        activeChannelId.set('other-room');

        pushMessage(ROOM, '@other:s', 'hello');

        expect(vi.mocked(sfx.playSfx)).toHaveBeenCalledWith('new_notif');
    });

    it('plays sound when app is unfocused even in the active room', () => {
        appFocused.set(false);
        activeChannelId.set(ROOM);

        pushMessage(ROOM, '@other:s', 'hello');

        expect(vi.mocked(sfx.playSfx)).toHaveBeenCalledWith('new_notif');
    });

    it('respects the 20-second cooldown between notification sounds', () => {
        appFocused.set(false);

        pushMessage(ROOM, '@other:s', 'first');
        expect(vi.mocked(sfx.playSfx)).toHaveBeenCalledWith('new_notif');
        vi.mocked(sfx.playSfx).mockClear();

        // Second message within 20s -- should not play
        vi.advanceTimersByTime(10_000);
        pushMessage(ROOM, '@other:s', 'second');
        expect(vi.mocked(sfx.playSfx)).not.toHaveBeenCalledWith('new_notif');

        // After 20s total -- should play again
        vi.advanceTimersByTime(10_000);
        pushMessage(ROOM, '@other:s', 'third');
        expect(vi.mocked(sfx.playSfx)).toHaveBeenCalledWith('new_notif');
    });

    it('does not play sound for non-Message timeline entries', () => {
        appFocused.set(false);

        handleMatrixEvent({
            type: 'TimelinePushBack',
            data: [ROOM, { sender: null, kind: 'ReadMarker' }],
        } as any);

        expect(vi.mocked(sfx.playSfx)).not.toHaveBeenCalledWith('new_notif');
    });

    it('does not play sound for StateEvent entries', () => {
        appFocused.set(false);

        handleMatrixEvent({
            type: 'TimelinePushBack',
            data: [ROOM, {
                sender: { display_name: 'Alice', avatar_url: null },
                kind: { StateEvent: { MemberJoined: { user_id: '@alice:s' } } },
            }],
        } as any);

        expect(vi.mocked(sfx.playSfx)).not.toHaveBeenCalledWith('new_notif');
    });
});
