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

// The router import pulls in every store module, which registers them.
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
    name: string;
    populate: () => void;
    expectCleared: () => void;
};

// Order matters: hiddenDmInfos populates via a ChannelList event that replaces the
// channel list, and messageWindows is read back last by selecting a channel.
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
            // The store is private, so observe it through unhide.
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
            setActiveChannel(ROOM_ID);
            expect(get(activeWindow).entries).toEqual([]);
        },
    },
];

// mumbleStatus goes last: its probe adds a user to voiceUsers.
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
            // localSession is private; a stale one would let a stranger reusing the ID
            // drive the mute toggles.
            fireMumbleEvent({ type: 'UserState', data: { ...makeVoiceUser(LOCAL_SESSION), self_mute: true } });
            expect(get(isMuted)).toBe(false);
        },
    },
];

describe('resetMatrixSession clears every registered store', () => {
    beforeEach(() => {
        resetStores();
    });

    it('has a probe for every registered matrix store', () => {
        expect(MATRIX_PROBES.map(p => p.name).sort()).toEqual(sessionStoreNames('matrix'));
    });

    it('returns every matrix store to its initial value on ServerReset', () => {
        for (const probe of MATRIX_PROBES) probe.populate();

        fireServerReset();

        for (const probe of MATRIX_PROBES) probe.expectCleared();
    });
});

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

describe('the voice session outlives a matrix reset', () => {
    beforeEach(() => {
        resetStores();
    });

    it('leaves voice state alone on ServerReset', () => {
        for (const probe of VOICE_PROBES) probe.populate();
        isMuted.set(true);
        isDeafened.set(true);

        fireServerReset();

        expect(get(mumbleStatus)).toBe('connected');
        expect(get(voiceChannels).size).toBe(1);
        expect(get(voiceUsers).size).toBe(1);
        expect(get(talkingUsers).size).toBe(1);
        expect(get(userVolumes)).toEqual({ someone: -3.5 });

        // The engine restores these itself.
        expect(get(isMuted)).toBe(true);
        expect(get(isDeafened)).toBe(true);
    });
});

// Pins scope membership, but cannot see a store nobody registered;
// storeClassification.test.ts covers that.
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
