import type { ChatMessage, SenderProfile, StateEventKind, TimelineEntry } from '$lib/types';

type StateLine = { id: string; text: string };

export type TimelineRow =
    | { type: 'message'; id: string; msg: ChatMessage; sender: SenderProfile | null; continuation: boolean }
    | { type: 'dayDivider'; id: string; timestamp: number }
    | { type: 'stateEvent'; id: string; text: string }
    | { type: 'stateGroup'; id: string; summary: string; lines: StateLine[] }
    | { type: 'undecryptable'; id: string; count: number };

const MIN_GROUP = 3;
const NAMES_SHOWN = 3;
const CONTINUATION_MS = 5 * 60 * 1000;

type Membership = 'joined' | 'left' | 'invited' | 'banned';

const VERB: Record<Membership, [one: string, many: string]> = {
    joined: ['joined', 'joined'],
    left: ['left', 'left'],
    invited: ['was invited', 'were invited'],
    banned: ['was banned', 'were banned'],
};

function memberName(userId: string): string {
    return userId.split(':')[0];
}

function membership(kind: StateEventKind): [Membership, string] | null {
    if (typeof kind === 'string') return null;
    if ('MemberJoined' in kind) return ['joined', kind.MemberJoined.user_id];
    if ('MemberLeft' in kind) return ['left', kind.MemberLeft.user_id];
    if ('MemberInvited' in kind) return ['invited', kind.MemberInvited.user_id];
    if ('MemberBanned' in kind) return ['banned', kind.MemberBanned.user_id];
    return null;
}

function stateEventText(kind: StateEventKind): string {
    if (typeof kind === 'string') return '';
    if ('RoomNameChanged' in kind) return `Room name changed to ${kind.RoomNameChanged.name}`;
    if ('RoomTopicChanged' in kind) return `Room topic changed to ${kind.RoomTopicChanged.topic}`;
    if ('RoomAvatarChanged' in kind) return 'Room avatar was changed';
    const member = membership(kind);
    if (!member) return '';
    const [change, userId] = member;
    return change === 'joined' || change === 'left'
        ? `${memberName(userId)} ${change} the room`
        : `${memberName(userId)} ${VERB[change][0]}`;
}

function continues(msg: ChatMessage, previous: TimelineEntry | undefined): boolean {
    if (!previous || typeof previous.kind !== 'object' || !('Message' in previous.kind)) return false;
    const prev = previous.kind.Message;
    return msg.sender === prev.sender && msg.timestamp - prev.timestamp < CONTINUATION_MS && !msg.edited;
}

/** The lines the timeline shows: runs of state events and of undecryptable messages fold into one row, and entries that show nothing neither appear nor break a run. */
export function timelineRows(entries: TimelineEntry[]): TimelineRow[] {
    const rows: TimelineRow[] = [];
    let states: { id: string; kind: StateEventKind }[] = [];
    let undecryptable: { type: 'undecryptable'; id: string; count: number } | null = null;

    function endRun() {
        if (states.length >= MIN_GROUP) {
            rows.push({
                type: 'stateGroup',
                id: states[0].id,
                summary: stateGroupSummary(states.map((s) => s.kind)),
                lines: states.map((s) => ({ id: s.id, text: stateEventText(s.kind) })),
            });
        } else {
            rows.push(...states.map((s) => ({ type: 'stateEvent' as const, id: s.id, text: stateEventText(s.kind) })));
        }
        if (undecryptable) rows.push(undecryptable);
        states = [];
        undecryptable = null;
    }

    let shown: TimelineEntry | undefined;
    entries.forEach((entry) => {
        const { id, kind } = entry;
        if (kind === 'Undecryptable') {
            if (states.length) endRun();
            if (undecryptable) undecryptable.count++;
            else undecryptable = { type: 'undecryptable', id, count: 1 };
        } else if (typeof kind === 'string') {
            return;
        } else if ('StateEvent' in kind) {
            if (!stateEventText(kind.StateEvent)) return;
            if (undecryptable) endRun();
            states.push({ id, kind: kind.StateEvent });
        } else if ('Message' in kind) {
            endRun();
            rows.push({
                type: 'message', id, msg: kind.Message, sender: entry.sender,
                continuation: continues(kind.Message, shown),
            });
        } else {
            endRun();
            rows.push({ type: 'dayDivider', id, timestamp: kind.DayDivider });
        }
        shown = entry;
    });
    endRun();
    return rows;
}

function listNames(names: string[]): string {
    const shown = names.length > NAMES_SHOWN ? names.slice(0, NAMES_SHOWN - 1) : names;
    const rest = names.length - shown.length;
    const parts = rest > 0 ? [...shown, `${rest} others`] : shown;
    return parts.length === 1 ? parts[0] : `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`;
}

function times(count: number): string {
    return count > 1 ? ` ${count} times` : '';
}

/** Each member's changes in order, members with the same changes listed together, then the room's own changes. */
function stateGroupSummary(kinds: StateEventKind[]): string {
    const changesByUser = new Map<string, Membership[]>();
    const roomChanges = { name: 0, topic: 0, avatar: 0 };

    for (const kind of kinds) {
        const member = membership(kind);
        if (member) {
            const [change, userId] = member;
            const changes = changesByUser.get(userId) ?? [];
            if (!changes.includes(change)) changes.push(change);
            changesByUser.set(userId, changes);
        } else if (typeof kind === 'object' && 'RoomNameChanged' in kind) roomChanges.name++;
        else if (typeof kind === 'object' && 'RoomTopicChanged' in kind) roomChanges.topic++;
        else if (typeof kind === 'object' && 'RoomAvatarChanged' in kind) roomChanges.avatar++;
    }

    const usersByChanges = new Map<string, { changes: Membership[]; names: string[] }>();
    for (const [userId, changes] of changesByUser) {
        const key = changes.join(' ');
        const group = usersByChanges.get(key) ?? { changes, names: [] };
        group.names.push(memberName(userId));
        usersByChanges.set(key, group);
    }

    const segments = [...usersByChanges.values()].map(({ changes, names }) => {
        const verbs = changes.map((change) => VERB[change][names.length === 1 ? 0 : 1]);
        return `${listNames(names)} ${verbs.join(' and ')}`;
    });
    if (roomChanges.name) segments.push(`the room name changed${times(roomChanges.name)}`);
    if (roomChanges.topic) segments.push(`the topic changed${times(roomChanges.topic)}`);
    if (roomChanges.avatar) segments.push(`the room avatar changed${times(roomChanges.avatar)}`);

    const summary = segments.join(', ');
    return summary.charAt(0).toUpperCase() + summary.slice(1);
}
