// Demo data injection for advertisement mockups.
//
// Usage: Developer Settings > Demo Mode > "Activate Demo Mode"
//
// Profile pictures and images are served from static/ via Vite.
// The mxcToUrl() helper in MessageGroup passes through non-mxc:// URLs.

import { channels } from '$lib/stores/channels';
import { activeChannelId, handleMatrixEvent as handleMessagesEvent } from '$lib/stores/messages';
import { currentUser } from '$lib/stores/user';
import { voiceChannels, voiceUsers, mumbleStatus } from '$lib/stores/voiceState';
import type { RoomInfo, TimelineEntry, ChatMessage, SenderProfile } from '$lib/types';
import type { VoiceChannel, VoiceUser } from '$lib/stores/voiceState';

// ---------------------------------------------------------------------------
// Time helpers
// ---------------------------------------------------------------------------

const NOW = Date.now();
const HOUR = 3_600_000;
const DAY = 86_400_000;
const YESTERDAY = NOW - DAY;

function t(base: number, hoursOffset: number, minutesOffset = 0): number {
    return base + hoursOffset * HOUR + minutesOffset * 60_000;
}

// ---------------------------------------------------------------------------
// Avatar helpers -- real images from static/, SVG fallback for the rest
// ---------------------------------------------------------------------------

function avatarSvg(initial: string, hue: number): string {
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40">`
        + `<rect width="40" height="40" rx="20" fill="hsl(${hue},55%,45%)"/>`
        + `<text x="20" y="21" text-anchor="middle" dominant-baseline="central" `
        + `fill="#fff" font-size="18" font-family="sans-serif" font-weight="600">`
        + `${initial}</text></svg>`;
    return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

const USER = {
    self:   '@nyx:etch.gg',
    kira:   '@kira:etch.gg',
    wren:   '@wren:etch.gg',
    sol:    '@sol:etch.gg',
    juno:   '@juno:etch.gg',
    reed:   '@reed:etch.gg',
    mika:   '@mika:etch.gg',
};

const AVATAR = {
    self: '/demo_shrek_pfp.png',           // real PFP
    kira: avatarSvg('K', 340),           // pink
    wren: '/demo_shibe_pfp.png',         // real PFP
    sol:  '/demo_capy_pfp.png',          // real PFP
    juno: avatarSvg('J', 210),           // blue
    reed: avatarSvg('R', 120),           // green
    mika: avatarSvg('M', 0),             // red
};

const PROFILE: Record<string, SenderProfile> = {
    self: { display_name: 'Nyx',  avatar_url: AVATAR.self },
    kira: { display_name: 'Kira', avatar_url: AVATAR.kira },
    wren: { display_name: 'Wren', avatar_url: AVATAR.wren },
    sol:  { display_name: 'Sol',  avatar_url: AVATAR.sol },
    juno: { display_name: 'Juno', avatar_url: AVATAR.juno },
    reed: { display_name: 'Reed', avatar_url: AVATAR.reed },
    mika: { display_name: 'Mika', avatar_url: AVATAR.mika },
};

// ---------------------------------------------------------------------------
// Room IDs
// ---------------------------------------------------------------------------

const ROOM = {
    general:  '!demo-general:etch.gg',
    dev:      '!demo-dev:etch.gg',
    random:   '!demo-random:etch.gg',
    lounge:   '!demo-voice-lounge:etch.gg',
    gaming:   '!demo-voice-gaming:etch.gg',
    dmKira:   '!demo-dm-kira:etch.gg',
};

// ---------------------------------------------------------------------------
// Entry builders
// ---------------------------------------------------------------------------

let msgIdCounter = 0;

function msg(
    sender: string,
    profile: SenderProfile,
    body: string,
    timestamp: number,
    opts: {
        html?: string;
        reactions?: Record<string, string[]>;
        media?: ChatMessage['media'];
    } = {},
): TimelineEntry {
    return {
        sender: profile,
        kind: {
            Message: {
                id: `$demo-${++msgIdCounter}`,
                sender,
                body,
                html_body: opts.html ?? null,
                media: opts.media ?? null,
                timestamp,
                reactions: opts.reactions ?? {},
            },
        },
    };
}

function divider(timestamp: number): TimelineEntry {
    return { sender: null, kind: { DayDivider: timestamp } };
}

// ---------------------------------------------------------------------------
// Rooms
// ---------------------------------------------------------------------------

const rooms: RoomInfo[] = [
    { id: ROOM.lounge,  display_name: 'Lounge',  etch_room_type: 'Voice', channel_id: 1, is_default: true,  unread_count: 0, is_encrypted: false },
    { id: ROOM.gaming,  display_name: 'Gaming',  etch_room_type: 'Voice', channel_id: 2, is_default: false, unread_count: 0, is_encrypted: false },
    { id: ROOM.general, display_name: 'general', etch_room_type: 'Text',  channel_id: null, is_default: false, unread_count: 0, is_encrypted: true },
    { id: ROOM.dev,     display_name: 'dev',     etch_room_type: 'Text',  channel_id: null, is_default: false, unread_count: 0, is_encrypted: true },
    { id: ROOM.random,  display_name: 'random',  etch_room_type: 'Text',  channel_id: null, is_default: false, unread_count: 2, is_encrypted: false },
    { id: ROOM.dmKira,  display_name: 'Kira',    etch_room_type: 'Dm',    channel_id: null, is_default: false, unread_count: 1, is_encrypted: true },
];

// ---------------------------------------------------------------------------
// #general messages (dino conversation)
// ---------------------------------------------------------------------------

const generalEntries: TimelineEntry[] = [
    divider(NOW),

    msg(USER.kira, PROFILE.kira, 'feathered dinos are making their way out to exhibits', t(NOW, -5, 43), {
        media: {
            mxc_url: '/demo_dino.png',
            mimetype: 'image/png',
            size: 465672,
            width: 480,
            height: 320,
            duration: 0,
        },
        reactions: { '\u{2764}\u{FE0F}': [USER.sol, USER.wren, USER.juno, USER.self] },
    }),
    msg(USER.sol, PROFILE.sol, 'FINALLY', t(NOW, -5, 22)),
    msg(USER.wren, PROFILE.wren, 'I was in ensenada in a small museum', t(NOW, -5, 10)),
    msg(USER.sol, PROFILE.sol, 'The third one especially looks really great.', t(NOW, -3, 59)),
    msg(USER.self, PROFILE.self, 'It found something really interesting in the grass', t(NOW, -3, 58)),
    msg(USER.self, PROFILE.self, 'Totally psyched out', t(NOW, -3, 57)),
    msg(USER.wren, PROFILE.wren, 'Like a dog with a chew toy', t(NOW, -3, 55)),
];

// ---------------------------------------------------------------------------
// #dev messages
// ---------------------------------------------------------------------------

const devEntries: TimelineEntry[] = [
    divider(YESTERDAY),

    msg(USER.kira, PROFILE.kira, 'anyone tried the new build yet?', t(YESTERDAY, 10)),
    msg(USER.sol, PROFILE.sol, 'yeah the voice quality is noticeably better', t(YESTERDAY, 10, 2)),
    msg(USER.wren, PROFILE.wren, 'agreed, latency feels lower too. whatever you changed with the codec is working', t(YESTERDAY, 10, 4)),
    msg(USER.kira, PROFILE.kira, 'nice. also pushed a fix for the echo cancellation issue, lmk if it comes back', t(YESTERDAY, 10, 6), {
        reactions: { '\u{1F44D}': [USER.sol, USER.wren, USER.self], '\u{1F389}': [USER.juno] },
    }),
    msg(USER.self, PROFILE.self, 'just tested it, sounds great on my end', t(YESTERDAY, 10, 12)),
    msg(USER.juno, PROFILE.juno, 'the UI update looks really clean btw', t(YESTERDAY, 10, 15), {
        reactions: { '\u{2764}\u{FE0F}': [USER.kira, USER.self] },
    }),

    divider(NOW),

    msg(USER.wren, PROFILE.wren, 'PR is up for the encrypted media fix. turned out the SDK caches the source separately from the event', t(NOW, -4)),
    msg(USER.kira, PROFILE.kira, 'looking at it now', t(NOW, -3, 50)),
    msg(USER.self, PROFILE.self, 'I can review after lunch', t(NOW, -3, 45)),
    msg(USER.sol, PROFILE.sol, 'also noticed we should debounce the resize observer, it fires a lot during image loads', t(NOW, -2)),
    msg(USER.kira, PROFILE.kira, "we suppress it during pagination already but yeah, general case could use a requestAnimationFrame guard", t(NOW, -1, 50)),
    msg(USER.wren, PROFILE.wren, "I'll add that to the PR", t(NOW, -1, 45), {
        reactions: { '\u{1F44D}': [USER.kira] },
    }),
];

// ---------------------------------------------------------------------------
// #random messages
// ---------------------------------------------------------------------------

const randomEntries: TimelineEntry[] = [
    divider(YESTERDAY),

    msg(USER.juno, PROFILE.juno, 'does anyone else name their test servers weird things or is that just me', t(YESTERDAY, 16)),
    msg(USER.reed, PROFILE.reed, 'my staging server is called "the void"', t(YESTERDAY, 16, 3)),
    msg(USER.mika, PROFILE.mika, "mine is 'oops'", t(YESTERDAY, 16, 5), {
        reactions: { '\u{1F602}': [USER.juno, USER.reed, USER.self, USER.kira] },
    }),
    msg(USER.self, PROFILE.self, "I just go with 'test' like a normal person", t(YESTERDAY, 16, 8)),
    msg(USER.juno, PROFILE.juno, 'boring', t(YESTERDAY, 16, 9), {
        reactions: { '\u{1F60F}': [USER.reed] },
    }),

    divider(NOW),

    msg(USER.mika, PROFILE.mika, 'hot take: dark mode should be the only mode', t(NOW, -5)),
    msg(USER.reed, PROFILE.reed, 'this is not a hot take, this is just being correct', t(NOW, -4, 55), {
        reactions: { '\u{1F525}': [USER.mika, USER.juno, USER.self], '\u{1F4AF}': [USER.wren] },
    }),
    msg(USER.juno, PROFILE.juno, 'light mode users are a threat to society', t(NOW, -4, 50)),
    msg(USER.sol, PROFILE.sol, 'I use light mode', t(NOW, -4, 45)),
    msg(USER.juno, PROFILE.juno, 'I said what I said', t(NOW, -4, 44), {
        reactions: { '\u{1F480}': [USER.reed, USER.mika, USER.self] },
    }),
];

// ---------------------------------------------------------------------------
// DM with Kira
// ---------------------------------------------------------------------------

const dmKiraEntries: TimelineEntry[] = [
    divider(NOW),

    msg(USER.kira, PROFILE.kira, 'hey, quick question about the voice reconnect logic', t(NOW, -1, 30)),
    msg(USER.kira, PROFILE.kira, 'when the bridge drops and comes back, are we re-resolving profiles or using cached ones?', t(NOW, -1, 28)),
    msg(USER.self, PROFILE.self, 'cached. the engine stores credentials from the first connect and reuses them', t(NOW, -1, 25)),
    msg(USER.kira, PROFILE.kira, "ok good, I was worried we'd hit the homeserver again for every user on reconnect", t(NOW, -1, 22)),
    msg(USER.self, PROFILE.self, "nah, profiles get resolved once on UserJoined and that's it", t(NOW, -1, 20)),
    msg(USER.kira, PROFILE.kira, 'perfect, thanks', t(NOW, -1, 18), {
        reactions: { '\u{1F44D}': [USER.self] },
    }),
];

// ---------------------------------------------------------------------------
// Voice channels and users
// ---------------------------------------------------------------------------

const demoVoiceChannels = new Map<number, VoiceChannel>([
    [0, { id: 0, name: 'Root', parent: -1 }],
    [1, { id: 1, name: 'Lounge', parent: 0 }],
    [2, { id: 2, name: 'Gaming', parent: 0 }],
]);

const demoVoiceUsers = new Map<number, VoiceUser>([
    [1, { session_id: 1, name: 'nyx',  display_name: 'Nyx',  avatar_url: AVATAR.self, channel_id: 1, muted: false, deafened: false, talking: false, hash: null }],
    [2, { session_id: 2, name: 'kira', display_name: 'Kira', avatar_url: AVATAR.kira, channel_id: 1, muted: false, deafened: false, talking: true,  hash: null }],
    [3, { session_id: 3, name: 'wren', display_name: 'Wren', avatar_url: AVATAR.wren, channel_id: 1, muted: true,  deafened: false, talking: false, hash: null }],
    [4, { session_id: 4, name: 'sol',  display_name: 'Sol',  avatar_url: AVATAR.sol,  channel_id: 2, muted: false, deafened: false, talking: false, hash: null }],
    [5, { session_id: 5, name: 'juno', display_name: 'Juno', avatar_url: AVATAR.juno, channel_id: 2, muted: false, deafened: true,  talking: false, hash: null }],
]);

// ---------------------------------------------------------------------------
// Injection
// ---------------------------------------------------------------------------

const messageMap: Record<string, TimelineEntry[]> = {
    [ROOM.general]: generalEntries,
    [ROOM.dev]:     devEntries,
    [ROOM.random]:  randomEntries,
    [ROOM.dmKira]:  dmKiraEntries,
};

export function injectDemoData(): void {
    // Current user
    currentUser.set({
        username: 'nyx',
        matrixId: USER.self,
        displayName: 'Nyx',
        avatarUrl: AVATAR.self,
    });

    // Channels
    channels.set(rooms);

    // Messages (inject via TimelineReset so the windows store picks them up)
    for (const [roomId, entries] of Object.entries(messageMap)) {
        handleMessagesEvent({ type: 'TimelineReset', data: [roomId, entries] });
        // Mark pagination as complete (no older history in demo)
        handleMessagesEvent({ type: 'PaginationComplete', data: [roomId, false] });
    }

    // Voice state
    mumbleStatus.set('connected');
    voiceChannels.set(demoVoiceChannels);
    voiceUsers.set(demoVoiceUsers);

    // Select #general as the starting view
    activeChannelId.set(ROOM.general);
}
