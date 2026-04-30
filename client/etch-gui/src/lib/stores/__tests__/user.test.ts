import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { currentUser, handleMatrixEvent, handleSystemEvent } from '../user';
import { resetStores } from './helpers';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

beforeEach(() => {
    resetStores();
});

describe('handleMatrixEvent (user)', () => {
    it('sets currentUser from CurrentUser event', () => {
        handleMatrixEvent({
            type: 'CurrentUser',
            data: {
                username: 'nyx',
                matrix_id: '@nyx:etch.gg',
                display_name: 'Nyx',
                avatar_url: 'http://avatar.png',
            },
        } as any);

        const user = get(currentUser);
        expect(user.username).toBe('nyx');
        expect(user.matrixId).toBe('@nyx:etch.gg');
        expect(user.displayName).toBe('Nyx');
        expect(user.avatarUrl).toBe('http://avatar.png');
    });

    it('handles null display_name and avatar_url', () => {
        handleMatrixEvent({
            type: 'CurrentUser',
            data: {
                username: 'nyx',
                matrix_id: '@nyx:etch.gg',
                display_name: null,
                avatar_url: null,
            },
        } as any);

        const user = get(currentUser);
        expect(user.displayName).toBeNull();
        expect(user.avatarUrl).toBeNull();
    });

    it('replaces previous user entirely', () => {
        handleMatrixEvent({
            type: 'CurrentUser',
            data: { username: 'old', matrix_id: '@old:s', display_name: 'Old', avatar_url: 'http://old' },
        } as any);
        handleMatrixEvent({
            type: 'CurrentUser',
            data: { username: 'new', matrix_id: '@new:s', display_name: 'New', avatar_url: null },
        } as any);

        const user = get(currentUser);
        expect(user.username).toBe('new');
        expect(user.matrixId).toBe('@new:s');
        expect(user.avatarUrl).toBeNull();
    });

    it('ignores non-CurrentUser events', () => {
        handleMatrixEvent({
            type: 'ChannelList',
            data: [],
        } as any);

        expect(get(currentUser).username).toBe('');
    });
});

describe('handleSystemEvent (user)', () => {
    beforeEach(() => {
        currentUser.set({
            username: 'nyx',
            matrixId: '@nyx:etch.gg',
            displayName: 'Old Name',
            avatarUrl: 'http://old-avatar',
        });
    });

    it('updates displayName and avatarUrl for matching username', () => {
        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'nyx', display_name: 'Nyx!', avatar_url: 'http://new' },
        } as any);

        const user = get(currentUser);
        expect(user.displayName).toBe('Nyx!');
        expect(user.avatarUrl).toBe('http://new');
    });

    it('preserves existing displayName when server sends null (fallback via ??)', () => {
        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'nyx', display_name: null, avatar_url: 'http://new' },
        } as any);

        const user = get(currentUser);
        expect(user.displayName).toBe('Old Name');
        expect(user.avatarUrl).toBe('http://new');
    });

    it('preserves existing avatarUrl when server sends null', () => {
        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'nyx', display_name: 'Updated', avatar_url: null },
        } as any);

        const user = get(currentUser);
        expect(user.displayName).toBe('Updated');
        expect(user.avatarUrl).toBe('http://old-avatar');
    });

    it('preserves both fields when server sends both null', () => {
        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'nyx', display_name: null, avatar_url: null },
        } as any);

        const user = get(currentUser);
        expect(user.displayName).toBe('Old Name');
        expect(user.avatarUrl).toBe('http://old-avatar');
    });

    it('does not update when username does not match', () => {
        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'someone_else', display_name: 'Hacked', avatar_url: 'http://evil' },
        } as any);

        const user = get(currentUser);
        expect(user.displayName).toBe('Old Name');
        expect(user.avatarUrl).toBe('http://old-avatar');
    });

    it('is a no-op when currentUser has empty username (not yet logged in)', () => {
        currentUser.set({ username: '', matrixId: '', displayName: null, avatarUrl: null });

        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'nyx', display_name: 'Nyx', avatar_url: null },
        } as any);

        expect(get(currentUser).displayName).toBeNull();
    });

    it('ignores non-UserProfileChanged events', () => {
        handleSystemEvent({
            type: 'LogError',
            data: { message: 'Error', target: 'test' },
        } as any);

        expect(get(currentUser).displayName).toBe('Old Name');
    });

    it('preserves username and matrixId (only updates profile fields)', () => {
        handleSystemEvent({
            type: 'UserProfileChanged',
            data: { username: 'nyx', display_name: 'New', avatar_url: 'http://new' },
        } as any);

        const user = get(currentUser);
        expect(user.username).toBe('nyx');
        expect(user.matrixId).toBe('@nyx:etch.gg');
    });
});
