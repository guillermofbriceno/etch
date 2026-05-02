import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import {
    loadSettings, connectToServer, addBookmark, updateBookmark, removeBookmark,
    serverBookmarks, selectedBookmarkId, connectingBookmark,
    handleSystemEvent, handleMatrixEvent, passwordRequested, matrixConnecting, mediaBaseUrl,
} from '../servers';
import { transmissionMode, vadThreshold, voiceHold, useMumbleSettings, deafenSuppressesNotifs } from '../voiceSettings';
import { activeOverlay } from '../overlay';
import { resetStores } from './helpers';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

beforeEach(() => {
    resetStores();
    vi.mocked(invoke).mockClear();
});

describe('servers IPC commands', () => {
    it('loadSettings sends System > LoadSettings', () => {
        loadSettings();

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: { type: 'System', data: { type: 'LoadSettings' } },
        });
    });

    describe('connectToServer', () => {
        it('sends System > ConnectToServer with mapped fields', async () => {
            const bookmark = {
                id: 'bk1', label: 'Test', address: 'example.com', port: 443,
                username: 'nyx', auto_connect: false,
                mumble_host: 'voice.example.com', mumble_port: 64738,
                mumble_username: 'nyx_voice', mumble_password: 'secret',
            };

            await connectToServer(bookmark, 'pass123');

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: {
                    type: 'System',
                    data: {
                        type: 'ConnectToServer',
                        data: {
                            username: 'nyx',
                            hostname: 'example.com',
                            port: '443',
                            password: 'pass123',
                            mumble_host: 'voice.example.com',
                            mumble_port: 64738,
                            mumble_username: 'nyx_voice',
                            mumble_password: 'secret',
                        },
                    },
                },
            });
        });

        it('sets connectingBookmark store', async () => {
            const bookmark = {
                id: 'bk1', label: 'Test', address: 'example.com', port: 443,
                username: 'nyx', auto_connect: false,
                mumble_host: null, mumble_port: null,
                mumble_username: null, mumble_password: null,
            };

            await connectToServer(bookmark);

            expect(get(connectingBookmark)).toEqual(bookmark);
        });

        it('defaults password to null', async () => {
            const bookmark = {
                id: 'bk1', label: 'Test', address: 'example.com', port: 443,
                username: 'nyx', auto_connect: false,
                mumble_host: null, mumble_port: null,
                mumble_username: null, mumble_password: null,
            };

            await connectToServer(bookmark);

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: {
                    type: 'System',
                    data: {
                        type: 'ConnectToServer',
                        data: expect.objectContaining({ password: null }),
                    },
                },
            });
        });

        it('converts port to string', async () => {
            const bookmark = {
                id: 'bk1', label: 'Test', address: 'example.com', port: 8448,
                username: 'nyx', auto_connect: false,
                mumble_host: null, mumble_port: null,
                mumble_username: null, mumble_password: null,
            };

            await connectToServer(bookmark);

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: {
                    type: 'System',
                    data: {
                        type: 'ConnectToServer',
                        data: expect.objectContaining({ port: '8448' }),
                    },
                },
            });
        });
    });

    describe('addBookmark', () => {
        it('sends System > SaveBookmarks', () => {
            addBookmark();

            expect(invoke).toHaveBeenCalledWith('core_command', expect.objectContaining({
                command: expect.objectContaining({
                    type: 'System',
                    data: expect.objectContaining({ type: 'SaveBookmarks' }),
                }),
            }));
        });

        it('adds a bookmark with correct default fields', () => {
            addBookmark();

            const bookmarks = get(serverBookmarks);
            expect(bookmarks).toHaveLength(1);
            expect(bookmarks[0]).toMatchObject({
                label: '',
                address: '',
                port: 443,
                username: '',
                auto_connect: false,
                mumble_host: null,
                mumble_port: null,
                mumble_username: null,
                mumble_password: null,
            });
            expect(bookmarks[0].id).toBeTruthy();
        });

        it('sets selectedBookmarkId to the new bookmark', () => {
            addBookmark();

            const bookmarks = get(serverBookmarks);
            expect(get(selectedBookmarkId)).toBe(bookmarks[0].id);
        });

        it('appends to existing bookmarks', () => {
            addBookmark();
            addBookmark();

            expect(get(serverBookmarks)).toHaveLength(2);
        });

        it('generates unique IDs for each bookmark', () => {
            addBookmark();
            addBookmark();

            const [bk1, bk2] = get(serverBookmarks);
            expect(bk1.id).not.toBe(bk2.id);
        });
    });

    describe('updateBookmark', () => {
        it('sends System > SaveBookmarks with updated list', () => {
            addBookmark();
            const [bookmark] = get(serverBookmarks);
            vi.mocked(invoke).mockClear();

            updateBookmark(bookmark.id, { label: 'My Server' });

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: {
                    type: 'System',
                    data: {
                        type: 'SaveBookmarks',
                        data: [expect.objectContaining({ id: bookmark.id, label: 'My Server' })],
                    },
                },
            });
        });

        it('updates the store with the new values', () => {
            addBookmark();
            const [bookmark] = get(serverBookmarks);

            updateBookmark(bookmark.id, { label: 'My Server', address: 'example.com', port: 8448 });

            const updated = get(serverBookmarks).find(b => b.id === bookmark.id);
            expect(updated?.label).toBe('My Server');
            expect(updated?.address).toBe('example.com');
            expect(updated?.port).toBe(8448);
        });

        it('does not modify other bookmarks', () => {
            addBookmark();
            addBookmark();
            const [bk1, bk2] = get(serverBookmarks);

            updateBookmark(bk1.id, { label: 'Updated' });

            expect(get(serverBookmarks).find(b => b.id === bk2.id)?.label).toBe('');
        });

        it('is a no-op for a nonexistent ID (still sends SaveBookmarks)', () => {
            addBookmark();
            vi.mocked(invoke).mockClear();

            updateBookmark('nonexistent', { label: 'Ghost' });

            // Still sends the save command, just no bookmark matches
            expect(invoke).toHaveBeenCalled();
            expect(get(serverBookmarks)).toHaveLength(1);
            expect(get(serverBookmarks)[0].label).toBe('');
        });
    });

    describe('removeBookmark', () => {
        it('sends System > SaveBookmarks with filtered list', () => {
            addBookmark();
            const [bookmark] = get(serverBookmarks);
            vi.mocked(invoke).mockClear();

            removeBookmark(bookmark.id);

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'System', data: { type: 'SaveBookmarks', data: [] } },
            });
        });

        it('removes the bookmark from the store', () => {
            addBookmark();
            const [bookmark] = get(serverBookmarks);

            removeBookmark(bookmark.id);

            expect(get(serverBookmarks)).toHaveLength(0);
        });

        it('clears selectedBookmarkId when removing the selected bookmark', () => {
            addBookmark();
            const [bookmark] = get(serverBookmarks);
            expect(get(selectedBookmarkId)).toBe(bookmark.id);

            removeBookmark(bookmark.id);

            expect(get(selectedBookmarkId)).toBeNull();
        });

        it('does not clear selectedBookmarkId when removing a non-selected bookmark', () => {
            addBookmark();
            addBookmark();
            const [bk1, bk2] = get(serverBookmarks);
            selectedBookmarkId.set(bk2.id);

            removeBookmark(bk1.id);

            expect(get(selectedBookmarkId)).toBe(bk2.id);
        });

        it('is a no-op for a nonexistent ID', () => {
            addBookmark();

            removeBookmark('nonexistent');

            expect(get(serverBookmarks)).toHaveLength(1);
        });
    });
});

describe('handleSystemEvent (SettingsLoaded)', () => {
    it('populates bookmarks', () => {
        const bookmark = {
            id: 'bk1', label: 'Test', address: 'example.com', port: 443,
            username: 'nyx', auto_connect: false,
            mumble_host: null, mumble_port: null, mumble_username: null, mumble_password: null,
        };

        handleSystemEvent({
            type: 'SettingsLoaded',
            data: {
                bookmarks: [bookmark],
                transmission_mode: null, vad_threshold: null,
                voice_hold: null, use_mumble_settings: null,
                hidden_dms: [],
            },
        } as any);

        expect(get(serverBookmarks)).toHaveLength(1);
        expect(get(serverBookmarks)[0].label).toBe('Test');
    });

    it('hydrates voice settings when values are provided', () => {
        handleSystemEvent({
            type: 'SettingsLoaded',
            data: {
                bookmarks: [],
                transmission_mode: 'push_to_talk',
                vad_threshold: 0.85,
                voice_hold: 400,
                use_mumble_settings: true,
                hidden_dms: [],
            },
        } as any);

        expect(get(transmissionMode)).toBe('push_to_talk');
        expect(get(vadThreshold)).toBe(85);
        expect(get(voiceHold)).toBe(400);
        expect(get(useMumbleSettings)).toBe(true);
    });

    it('leaves voice settings at defaults when values are null', () => {
        handleSystemEvent({
            type: 'SettingsLoaded',
            data: {
                bookmarks: [],
                transmission_mode: null,
                vad_threshold: null,
                voice_hold: null,
                use_mumble_settings: null,
                hidden_dms: [],
            },
        } as any);

        expect(get(transmissionMode)).toBe('voice_activation');
        expect(get(vadThreshold)).toBe(60);
        expect(get(voiceHold)).toBe(250);
        expect(get(useMumbleSettings)).toBe(false);
    });

    it('hydrates deafenSuppressesNotifs when provided', () => {
        handleSystemEvent({
            type: 'SettingsLoaded',
            data: {
                bookmarks: [],
                transmission_mode: null, vad_threshold: null,
                voice_hold: null, use_mumble_settings: null,
                deafen_suppresses_notifs: false,
                hidden_dms: [],
            },
        } as any);

        expect(get(deafenSuppressesNotifs)).toBe(false);
    });

    it('defaults deafenSuppressesNotifs to true when null', () => {
        handleSystemEvent({
            type: 'SettingsLoaded',
            data: {
                bookmarks: [],
                transmission_mode: null, vad_threshold: null,
                voice_hold: null, use_mumble_settings: null,
                deafen_suppresses_notifs: null,
                hidden_dms: [],
            },
        } as any);

        expect(get(deafenSuppressesNotifs)).toBe(true);
    });

    it('sets connectingBookmark for an auto-connect bookmark', () => {
        const autoBookmark = {
            id: 'bk1', label: 'Auto', address: 'auto.example.com', port: 443,
            username: 'nyx', auto_connect: true,
            mumble_host: null, mumble_port: null, mumble_username: null, mumble_password: null,
        };

        handleSystemEvent({
            type: 'SettingsLoaded',
            data: {
                bookmarks: [autoBookmark],
                transmission_mode: null, vad_threshold: null,
                voice_hold: null, use_mumble_settings: null,
                hidden_dms: [],
            },
        } as any);

        expect(get(connectingBookmark)?.id).toBe('bk1');
    });

    it('does not set connectingBookmark when no bookmark has auto_connect', () => {
        handleSystemEvent({
            type: 'SettingsLoaded',
            data: {
                bookmarks: [
                    { id: 'bk1', label: 'Manual', address: 'a.com', port: 443, username: 'u', auto_connect: false, mumble_host: null, mumble_port: null, mumble_username: null, mumble_password: null },
                ],
                transmission_mode: null, vad_threshold: null,
                voice_hold: null, use_mumble_settings: null,
                hidden_dms: [],
            },
        } as any);

        expect(get(connectingBookmark)).toBeNull();
    });

    it('ignores non-SettingsLoaded events', () => {
        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'nyx', display_name: 'Nyx', avatar_url: null },
        } as any);

        expect(get(serverBookmarks)).toHaveLength(0);
    });
});

describe('handleMatrixEvent (servers)', () => {
    it('sets passwordRequested on PasswordRequest', () => {
        handleMatrixEvent({ type: 'PasswordRequest' } as any);

        expect(get(passwordRequested)).toBe(true);
    });

    it('sets mediaBaseUrl on HomeserverResolved', () => {
        handleMatrixEvent({ type: 'HomeserverResolved', data: 'https://matrix.etch.gg' } as any);

        expect(get(mediaBaseUrl)).toBe('https://matrix.etch.gg');
    });

    it('sets matrixConnecting to true on Connecting', () => {
        handleMatrixEvent({ type: 'ConnectionState', data: { type: 'Connecting' } } as any);

        expect(get(matrixConnecting)).toBe(true);
    });

    it('sets matrixConnecting to false on Connected', () => {
        matrixConnecting.set(true);

        handleMatrixEvent({ type: 'ConnectionState', data: { type: 'Connected' } } as any);

        expect(get(matrixConnecting)).toBe(false);
    });

    it('closes overlay on Connected', () => {
        activeOverlay.set('connect');

        handleMatrixEvent({ type: 'ConnectionState', data: { type: 'Connected' } } as any);

        expect(get(activeOverlay)).toBe('none');
    });
});
