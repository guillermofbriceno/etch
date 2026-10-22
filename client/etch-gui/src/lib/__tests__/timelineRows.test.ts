import { describe, it, expect } from 'vitest';
import type { StateEventKind, TimelineEntry } from '$lib/types';
import { timelineRows } from '../timelineRows';

let nextId = 0;
const state = (kind: StateEventKind): TimelineEntry => ({ id: `item-${++nextId}`, sender: null, kind: { StateEvent: kind } });
const joined = (user: string) => state({ MemberJoined: { user_id: `@${user}:s` } });
const left = (user: string) => state({ MemberLeft: { user_id: `@${user}:s` } });
const invited = (user: string) => state({ MemberInvited: { user_id: `@${user}:s` } });
const renamed = (name: string) => state({ RoomNameChanged: { name } });

function message(sender: string, minute: number, edited = false): TimelineEntry {
    return {
        id: `item-${++nextId}`,
        sender: null,
        kind: {
            Message: {
                id: `$${nextId}`, sender, body: '', html_body: null, media: null,
                timestamp: minute * 60_000, edited, reactions: {},
            },
        },
    };
}

describe('the summary of a run of state events', () => {
    it.each<[string, TimelineEntry[], string]>([
        ['names at most three members, then counts the rest',
            ['a', 'b', 'c', 'd', 'e'].map(joined), '@a, @b and 3 others joined'],
        ['lists members with the same changes together, in order',
            [joined('a'), left('b'), joined('c'), left('a'), left('c')], '@a and @c joined and left, @b left'],
        ['agrees the verb with the number of members',
            [invited('a'), invited('b'), joined('a'), joined('b'), invited('c')], '@a and @b were invited and joined, @c was invited'],
        ['counts room changes after the members',
            [renamed('one'), joined('a'), renamed('two')], '@a joined, the room name changed 2 times'],
        ['starts with a capital when a room change comes first',
            [renamed('one'), renamed('two'), renamed('three')], 'The room name changed 3 times'],
    ])('%s', (_, entries, expected) => {
        expect(timelineRows(entries)).toEqual([expect.objectContaining({ type: 'stateGroup', summary: expected })]);
    });
});

describe('message continuation', () => {
    it('continues a sender only right after their own recent, unedited message, past what shows nothing', () => {
        const rows = timelineRows([
            message('@a:s', 0),
            { id: 'marker', sender: null, kind: 'ReadMarker' },
            message('@a:s', 4),
            message('@b:s', 5),
            message('@b:s', 11),
            message('@b:s', 12, true),
        ]);

        expect(rows.map((row) => row.type === 'message' && row.continuation)).toEqual([
            false, true, false, false, false,
        ]);
    });
});
