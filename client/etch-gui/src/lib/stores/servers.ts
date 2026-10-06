import { writable, get } from 'svelte/store';
import type { ServerBookmark } from '$lib/types';
import type { MatrixEvent, SystemEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import { invoke } from '@tauri-apps/api/core';
import { closeOverlay } from './overlay';
import { initHiddenDms } from './channels';
import { transmissionMode, vadThreshold, voiceHold, useMumbleSettings, deafenSuppressesNotifs } from './voiceSettings';
import type { TransmissionMode } from './voiceSettings';
import { registerSessionStore, declareStores } from './session';

export const serverBookmarks = writable<ServerBookmark[]>([]);
export const selectedBookmarkId = writable<string | null>(null);

// Set before ServerReset arrives as the reply to connecting, so a reset must not clear it.
export const connectingBookmark = writable<ServerBookmark | null>(null);

export const signOutDialogOpen = writable<boolean>(false);
// A failed sign out reconnects, and the reset that comes with it must not take the reason away.
export const signOutError = writable<string | null>(null);

declareStores('device', 'serverBookmarks', 'selectedBookmarkId', 'connectingBookmark', 'signOutDialogOpen', 'signOutError');

export const passwordRequested = writable<boolean>(false);
export const matrixConnecting = writable<boolean>(false);
export const matrixConnected = writable<boolean>(false);
export const mediaBaseUrl = writable<string | null>(null);
// A sign out is out and core has answered with neither SignedOut nor SignOutFailed.
export const signingOut = writable<boolean>(false);

export function loadSettings(): void {
    sendCoreCommand({ type: 'System', data: { type: 'LoadSettings' } });
}

function saveBookmarks(bookmarks: ServerBookmark[]): void {
    sendCoreCommand({
        type: 'System',
        data: { type: 'SaveBookmarks', data: bookmarks },
    });
}

export async function connectToServer(bookmark: ServerBookmark, password: string | null = null): Promise<void> {
    connectingBookmark.set(bookmark);
    await sendCoreCommand({
        type: 'System',
        data: {
            type: 'ConnectToServer',
            data: {
                username: bookmark.username,
                hostname: bookmark.address,
                port: String(bookmark.port),
                password,
                mumble_host: bookmark.mumble_host,
                mumble_port: bookmark.mumble_port,
                mumble_username: bookmark.mumble_username,
                mumble_password: bookmark.mumble_password,
            },
        },
    });
}

export function openSignOut(): void {
    signOutError.set(null);
    signOutDialogOpen.set(true);
}

export function closeSignOut(): void {
    signOutDialogOpen.set(false);
    signOutError.set(null);
}

export function signOut(): void {
    signOutError.set(null);
    signingOut.set(true);
    sendCoreCommand({ type: 'System', data: { type: 'SignOut' } }).catch((e) => {
        signingOut.set(false);
        signOutError.set(`Etch could not send the request: ${e}`);
    });
}

export function addBookmark(): void {
    const bookmark: ServerBookmark = {
        id: crypto.randomUUID(),
        label: '',
        address: '',
        port: 443,
        username: '',
        auto_connect: false,
        mumble_host: null,
        mumble_port: null,
        mumble_username: null,
        mumble_password: null,
    };
    serverBookmarks.update(list => {
        const updated = [...list, bookmark];
        saveBookmarks(updated);
        return updated;
    });
    selectedBookmarkId.set(bookmark.id);
}

export function updateBookmark(id: string, fields: Partial<Omit<ServerBookmark, 'id'>>): void {
    serverBookmarks.update(list => {
        const updated = list.map(b => b.id === id ? { ...b, ...fields } : b);
        saveBookmarks(updated);
        return updated;
    });
}

export function removeBookmark(id: string): void {
    serverBookmarks.update(list => {
        const updated = list.filter(b => b.id !== id);
        saveBookmarks(updated);
        return updated;
    });
    if (get(selectedBookmarkId) === id) {
        selectedBookmarkId.set(null);
    }
}

const clearMediaBaseUrl = (): void => { mediaBaseUrl.set(null); };
const clearPasswordRequested = (): void => { passwordRequested.set(false); };
const clearMatrixConnecting = (): void => { matrixConnecting.set(false); };
const clearMatrixConnected = (): void => { matrixConnected.set(false); };

registerSessionStore('matrix', 'mediaBaseUrl', clearMediaBaseUrl);
registerSessionStore('matrix', 'passwordRequested', clearPasswordRequested);
registerSessionStore('matrix', 'matrixConnecting', clearMatrixConnecting);
registerSessionStore('matrix', 'matrixConnected', clearMatrixConnected);
registerSessionStore('matrix', 'signingOut', () => { signingOut.set(false); });

// Handlers called by eventRouter
export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type === 'PasswordRequest') {
        passwordRequested.set(true);
    } else if (me.type === 'HomeserverResolved') {
        mediaBaseUrl.set(me.data);
    } else if (me.type === 'ConnectionState') {
        matrixConnecting.set(me.data.type === 'Connecting');
        matrixConnected.set(me.data.type === 'Connected');
        if (me.data.type === 'Connected') {
            closeOverlay();
        }
    }
}

export function handleSystemEvent(se: SystemEvent): void {
    if (se.type === 'SignedOut') {
        // The ServerReset before it cleared the session, but an overlay and the dialog are kept across a reset.
        closeOverlay();
        closeSignOut();
    } else if (se.type === 'SignOutFailed') {
        signingOut.set(false);
        signOutError.set(se.data.reason);
    } else if (se.type === 'SettingsLoaded') {
        serverBookmarks.set(se.data.bookmarks);
        if (se.data.transmission_mode != null) transmissionMode.set(se.data.transmission_mode as TransmissionMode);
        if (se.data.vad_threshold != null) vadThreshold.set(Math.round(se.data.vad_threshold * 100));
        if (se.data.voice_hold != null) voiceHold.set(se.data.voice_hold);
        useMumbleSettings.set(se.data.use_mumble_settings ?? false);
        deafenSuppressesNotifs.set(se.data.deafen_suppresses_notifs ?? true);
        initHiddenDms(se.data.hidden_dms ?? []);
        // Mirror the backend's auto-connect: set the active bookmark so mediaBaseUrl resolves
        const autoConnect = se.data.bookmarks.find(b => b.auto_connect);
        if (autoConnect) {
            connectingBookmark.set(autoConnect);
        }

        if (se.data.custom_css) {
            invoke('load_custom_css', { path: se.data.custom_css })
                .then((css) => {
                    let el = document.getElementById('custom-user-css');
                    if (!el) {
                        el = document.createElement('style');
                        el.id = 'custom-user-css';
                        document.head.appendChild(el);
                    }
                    el.textContent = css as string;
                })
                .catch((e) => console.warn('[css] Failed to load custom stylesheet:', e));
        }
    }
}
