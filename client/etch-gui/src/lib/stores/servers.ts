import { writable, get } from 'svelte/store';
import type { ServerBookmark } from '$lib/types';
import type { MatrixEvent, SystemEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import { invoke } from '@tauri-apps/api/core';
import { closeOverlay } from './overlay';
import { initHiddenDms } from './channels';
import { transmissionMode, vadThreshold, voiceHold, useMumbleSettings, deafenSuppressesNotifs } from './voiceSettings';
import type { TransmissionMode } from './voiceSettings';
import { registerSessionStore } from './session';

// Device-scoped. The saved server list and the row selected in settings are
// both preferences that outlive any one connection.
export const serverBookmarks = writable<ServerBookmark[]>([]);
export const selectedBookmarkId = writable<string | null>(null);

// Device-scoped, despite naming a server. This is the bookmark of the session
// being opened, set by connectToServer() before the command leaves the app,
// and ServerReset arrives afterwards as the backend's reply. Clearing it here
// would wipe the details of the connection currently in flight and leave the
// password prompt with nothing to show.
export const connectingBookmark = writable<ServerBookmark | null>(null);

// Matrix session. All three describe the homeserver connection being
// established or already attached. ServerReset is emitted before the
// Connecting state for the new attempt, so clearing them cannot race the
// connection that follows.
export const passwordRequested = writable<boolean>(false);
export const matrixConnecting = writable<boolean>(false);
export const mediaBaseUrl = writable<string | null>(null);

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

registerSessionStore('matrix', 'mediaBaseUrl', clearMediaBaseUrl);
registerSessionStore('matrix', 'passwordRequested', clearPasswordRequested);
registerSessionStore('matrix', 'matrixConnecting', clearMatrixConnecting);

/** Clear the connection state this module owns. resetMatrixSession() already
 *  does this as part of a full ServerReset; this stays exported for callers
 *  that want the connection half on its own. */
export function resetServerConnection(): void {
    clearMediaBaseUrl();
    clearPasswordRequested();
    clearMatrixConnecting();
}

// Handlers called by eventRouter
export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type === 'PasswordRequest') {
        passwordRequested.set(true);
    } else if (me.type === 'HomeserverResolved') {
        mediaBaseUrl.set(me.data);
    } else if (me.type === 'ConnectionState') {
        matrixConnecting.set(me.data.type === 'Connecting');
        if (me.data.type === 'Connected') {
            closeOverlay();
        }
    }
}

export function handleSystemEvent(se: SystemEvent): void {
    if (se.type === 'SettingsLoaded') {
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
