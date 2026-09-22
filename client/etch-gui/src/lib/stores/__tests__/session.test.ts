import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { listen } from '@tauri-apps/api/event';
import { resetStores } from './helpers';
import type { CoreEvent } from '$lib/ipc';
import type { RoomInfo, RoomType } from '$lib/types';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

// Importing the router is what pulls every store module into the graph, which
// is what makes them register. This mirrors how the running app loads them,
// so a module the router cannot reach shows up here as a missing name rather
// than as a store that quietly never gets cleared.
import { initEventRouter } from '../eventRouter';
import { sessionStoreNames } from '../session';

import { activeChannelId } from '../activeChannel';
import { isMuted, isDeafened } from '../audio';
import { channels, dmLastActivity, initHiddenDms, unhideDm } from '../channels';
import { replyingTo, editingMessage } from '../compose';
import { activeWindow, setActiveChannel } from '../messages';
import { mediaBaseUrl, passwordRequested, matrixConnecting } from '../servers';
import { currentUser } from '../user';
import { userVolumes } from '../userVolumes';
import { voiceChannels, voiceUsers, talkingUsers, mumbleStatus, certChangeRequest } from '../voiceState';

// Capture the core_event callback registered by initEventRouter.
type ListenCallback = (event: { payload: any }) => void;

function getListenCallback(eventName: string): ListenCallback {
    const calls = vi.mocked(listen).mock.calls;
    const match = calls.find(c => c[0] === eventName);
    if (!match) throw new Error(`No listen() call found for event "${eventName}"`);
    return match[1] as ListenCallback;
}

let routeCoreEvent: ListenCallback;

vi.mocked(listen).mockClear();
initEventRouter();
routeCoreEvent = getListenCallback('core_event');

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

function fireMumbleEvent(data: any): void {
    routeCoreEvent({
        payload: { type: 'Mumble', data } satisfies CoreEvent,
    });
}

function fireVoiceDisconnect(): void {
    fireMumbleEvent({ type: 'ConnectionState', data: { type: 'Disconnected' } });
}

const ROOM_ID = '!room:example.org';
const HIDDEN_DM_ID = '!hidden:example.org';
const LOCAL_SESSION = 7;

function makeRoom(id: string, name: string, kind: RoomType): RoomInfo {
    return {
        id,
        display_name: name,
        etch_room_type: kind,
        channel_id: null,
        is_default: false,
        unread_count: 0,
        is_encrypted: false,
        avatar_url: null,
    };
}

function makeMessageEntry(id: string, body: string) {
    return {
        sender: { display_name: 'Someone', avatar_url: null },
        kind: {
            Message: {
                id, sender: '@someone:example.org', body, html_body: null,
                media: null, timestamp: 1000, edited: false, reactions: {},
            },
        },
    };
}

function makeChatMessage(id: string) {
    return {
        id, sender: '@someone:example.org', body: 'hello', html_body: null,
        media: null, timestamp: 1000, edited: false, reactions: {},
    };
}

function makeVoiceUser(sessionId: number) {
    return {
        session_id: sessionId, name: 'someone', display_name: null, avatar_url: null,
        channel_id: 1, muted: false, deafened: false, hash: null,
    };
}

type SessionStoreProbe = {
    /** The name the store is registered under in session.ts. */
    name: string;
    /** Put the store into a state a live session would produce. */
    populate: () => void;
    /** Assert the store holds what it held before that session began. */
    expectCleared: () => void;
};

// Both phases run in array order. Two stores here are private to their module
// and can only be observed through a store that is itself being reset, so
// their probes reach past their own store and their position matters:
// hiddenDmInfos populates through a ChannelList event that replaces the whole
// channel list, so it goes first; messageWindows can only be read back by
// selecting a channel again, so it goes last.
const MATRIX_PROBES: SessionStoreProbe[] = [
    {
        name: 'hiddenDmInfos',
        populate: () => {
            initHiddenDms([HIDDEN_DM_ID]);
            fireMatrixEvent({
                type: 'ChannelList',
                data: [makeRoom(HIDDEN_DM_ID, 'Hidden DM', 'Dm')],
            });
        },
        expectCleared: () => {
            // The stash is private. With no cached RoomInfo behind it, an
            // unhide has nothing to put back and the list stays empty.
            unhideDm(HIDDEN_DM_ID);
            expect(get(channels)).toEqual([]);
        },
    },
    {
        name: 'channels',
        populate: () => { channels.set([makeRoom(ROOM_ID, 'General', 'Text')]); },
        expectCleared: () => { expect(get(channels)).toEqual([]); },
    },
    {
        name: 'dmLastActivity',
        populate: () => { dmLastActivity.set({ [ROOM_ID]: 1000 }); },
        expectCleared: () => { expect(get(dmLastActivity)).toEqual({}); },
    },
    {
        name: 'activeChannelId',
        populate: () => { activeChannelId.set(ROOM_ID); },
        expectCleared: () => { expect(get(activeChannelId)).toBeNull(); },
    },
    {
        name: 'currentUser',
        populate: () => {
            currentUser.set({
                username: 'someone',
                matrixId: '@someone:example.org',
                displayName: 'Someone',
                avatarUrl: 'mxc://example.org/avatar',
            });
        },
        expectCleared: () => {
            expect(get(currentUser)).toEqual({
                username: '', matrixId: '', displayName: null, avatarUrl: null,
            });
        },
    },
    {
        name: 'replyingTo',
        populate: () => { replyingTo.set(makeChatMessage('$reply')); },
        expectCleared: () => { expect(get(replyingTo)).toBeNull(); },
    },
    {
        name: 'editingMessage',
        populate: () => { editingMessage.set(makeChatMessage('$edit')); },
        expectCleared: () => { expect(get(editingMessage)).toBeNull(); },
    },
    {
        name: 'mediaBaseUrl',
        populate: () => { mediaBaseUrl.set('https://example.org'); },
        expectCleared: () => { expect(get(mediaBaseUrl)).toBeNull(); },
    },
    {
        name: 'passwordRequested',
        populate: () => { passwordRequested.set(true); },
        expectCleared: () => { expect(get(passwordRequested)).toBe(false); },
    },
    {
        name: 'matrixConnecting',
        populate: () => { matrixConnecting.set(true); },
        expectCleared: () => { expect(get(matrixConnecting)).toBe(false); },
    },
    {
        name: 'certChangeRequest',
        populate: () => {
            certChangeRequest.set({ host: 'voice.example.org', port: 64738, new_fingerprint: 'aa:bb:cc' });
        },
        expectCleared: () => { expect(get(certChangeRequest)).toBeNull(); },
    },
    {
        name: 'messageWindows',
        populate: () => {
            fireMatrixEvent({
                type: 'TimelinePushBack',
                data: [ROOM_ID, makeMessageEntry('$msg', 'hello from the old session')],
            });
        },
        expectCleared: () => {
            // The window map is private, so read it back the only way a
            // component can: select the room and look at activeWindow.
            setActiveChannel(ROOM_ID);
            expect(get(activeWindow).entries).toEqual([]);
        },
    },
];

// mumbleStatus goes last: its probe fires a UserState to prove the private
// localSession went with it, and that adds a user to voiceUsers.
const VOICE_PROBES: SessionStoreProbe[] = [
    {
        name: 'userVolumes',
        populate: () => { userVolumes.set({ someone: -3.5 }); },
        expectCleared: () => { expect(get(userVolumes)).toEqual({}); },
    },
    {
        name: 'voiceChannels',
        populate: () => { voiceChannels.set(new Map([[1, { id: 1, name: 'Root', parent: 0 }]])); },
        expectCleared: () => { expect(get(voiceChannels).size).toBe(0); },
    },
    {
        name: 'voiceUsers',
        populate: () => { voiceUsers.set(new Map([[LOCAL_SESSION, makeVoiceUser(LOCAL_SESSION)]])); },
        expectCleared: () => { expect(get(voiceUsers).size).toBe(0); },
    },
    {
        name: 'talkingUsers',
        populate: () => { talkingUsers.set(new Set([LOCAL_SESSION])); },
        expectCleared: () => { expect(get(talkingUsers).size).toBe(0); },
    },
    {
        name: 'mumbleStatus',
        populate: () => {
            fireMumbleEvent({ type: 'LocalSession', data: LOCAL_SESSION });
            mumbleStatus.set('connected');
        },
        expectCleared: () => {
            expect(get(mumbleStatus)).toBe('disconnected');
            // localSession is private. Prove it went too: a UserState for the
            // old session ID must no longer be taken for the local user, or a
            // stranger on the next server reusing that ID would drive the
            // local mute toggles.
            fireMumbleEvent({ type: 'UserState', data: { ...makeVoiceUser(LOCAL_SESSION), self_mute: true } });
            expect(get(isMuted)).toBe(false);
        },
    },
];

// -----------------------------------------------------------------------
// Matrix session scope
// -----------------------------------------------------------------------

describe('resetMatrixSession clears every registered store', () => {
    beforeEach(() => {
        resetStores();
    });

    it('has a probe for every registered matrix store', () => {
        // Without this the test below would quietly stop covering a store the
        // moment one was registered without a probe to go with it.
        expect(MATRIX_PROBES.map(p => p.name).sort()).toEqual(sessionStoreNames('matrix'));
    });

    it('returns every matrix store to its initial value on ServerReset', () => {
        for (const probe of MATRIX_PROBES) probe.populate();

        fireServerReset();

        for (const probe of MATRIX_PROBES) probe.expectCleared();
    });
});

// -----------------------------------------------------------------------
// Voice session scope
// -----------------------------------------------------------------------

describe('resetVoiceSession clears every registered store', () => {
    beforeEach(() => {
        resetStores();
    });

    it('has a probe for every registered voice store', () => {
        expect(VOICE_PROBES.map(p => p.name).sort()).toEqual(sessionStoreNames('voice'));
    });

    it('returns every voice store to its initial value on a Mumble disconnect', () => {
        for (const probe of VOICE_PROBES) probe.populate();

        fireVoiceDisconnect();

        for (const probe of VOICE_PROBES) probe.expectCleared();
    });
});

// -----------------------------------------------------------------------
// The two lifecycles stay independent
// -----------------------------------------------------------------------

describe('the voice session outlives a matrix reset', () => {
    beforeEach(() => {
        resetStores();
    });

    it('leaves voice state alone on ServerReset', () => {
        // ServerReset fires on every Matrix connect attempt, retries in the
        // backoff loop included, and the engine deliberately keeps Mumble
        // joined when the voice server has not moved. Nothing reconnects to
        // repopulate these, so clearing them here would blank the voice panel
        // of a user who is still in the channel and still audible.
        for (const probe of VOICE_PROBES) probe.populate();
        isMuted.set(true);
        isDeafened.set(true);

        fireServerReset();

        expect(get(mumbleStatus)).toBe('connected');
        expect(get(voiceChannels).size).toBe(1);
        expect(get(voiceUsers).size).toBe(1);
        expect(get(talkingUsers).size).toBe(1);
        expect(get(userVolumes)).toEqual({ someone: -3.5 });

        // The engine persists these in VoiceSessionState and re-sends them
        // when Mumble comes back up, so the frontend must not second-guess it.
        expect(get(isMuted)).toBe(true);
        expect(get(isDeafened)).toBe(true);
    });
});

// -----------------------------------------------------------------------
// The registered sets themselves
// -----------------------------------------------------------------------

// These two lists exist to fail. They are the only thing standing between a
// new session-scoped store and the old bug, where state belonging to a session
// that had ended kept showing because nobody remembered to clear it.
//
// If one fails, work out which bucket the store belongs in before touching the
// list:
//   matrix session  state from one homeserver connection, cleared on ServerReset
//   voice session   state from one Mumble connection, cleared on its disconnect
//   device-scoped   a preference or UI state that has to survive both
//   backend-owned   the engine persists and replays it, so the frontend must
//                   not clear it (isMuted and isDeafened are the two)
//   derived         computed from other stores, needs no reset of its own
//
// Register it in the right scope and add it here, or decide it belongs in
// neither and say so in a comment where it is defined. Editing a list to make
// the red go away puts the bug back.
const EXPECTED_MATRIX_STORES = [
    'activeChannelId',
    'certChangeRequest',
    'channels',
    'currentUser',
    'dmLastActivity',
    'editingMessage',
    'hiddenDmInfos',
    'matrixConnecting',
    'mediaBaseUrl',
    'messageWindows',
    'passwordRequested',
    'replyingTo',
];

const EXPECTED_VOICE_STORES = [
    'mumbleStatus',
    'talkingUsers',
    'userVolumes',
    'voiceChannels',
    'voiceUsers',
];

describe('session store registry', () => {
    it('registers exactly the stores on the matrix session list', () => {
        expect(sessionStoreNames('matrix')).toEqual(EXPECTED_MATRIX_STORES);
    });

    it('registers exactly the stores on the voice session list', () => {
        expect(sessionStoreNames('voice')).toEqual(EXPECTED_VOICE_STORES);
    });
});
