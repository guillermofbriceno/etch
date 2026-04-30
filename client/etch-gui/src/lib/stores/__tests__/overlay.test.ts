import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { activeOverlay, overlayImageUrl, settingsTab, openSettings, openImage, openConnect, closeOverlay } from '../overlay';
import { resetStores } from './helpers';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

beforeEach(() => {
    resetStores();
});

describe('overlay state machine', () => {
    it('starts with no overlay active', () => {
        expect(get(activeOverlay)).toBe('none');
        expect(get(overlayImageUrl)).toBeNull();
    });

    describe('openSettings', () => {
        it('sets activeOverlay to settings', () => {
            openSettings();

            expect(get(activeOverlay)).toBe('settings');
        });

        it('defaults settingsTab to voice', () => {
            openSettings();

            expect(get(settingsTab)).toBe('voice');
        });

        it('sets settingsTab to the specified tab', () => {
            openSettings('audio');

            expect(get(settingsTab)).toBe('audio');
        });

        it('can switch tabs by calling again', () => {
            openSettings('voice');
            openSettings('audio');

            expect(get(settingsTab)).toBe('audio');
            expect(get(activeOverlay)).toBe('settings');
        });
    });

    describe('openImage', () => {
        it('sets activeOverlay to image and stores the URL', () => {
            openImage('https://example.com/pic.png');

            expect(get(activeOverlay)).toBe('image');
            expect(get(overlayImageUrl)).toBe('https://example.com/pic.png');
        });

        it('replaces a previously opened image', () => {
            openImage('https://example.com/first.png');
            openImage('https://example.com/second.png');

            expect(get(overlayImageUrl)).toBe('https://example.com/second.png');
        });
    });

    describe('openConnect', () => {
        it('sets activeOverlay to connect', () => {
            openConnect();

            expect(get(activeOverlay)).toBe('connect');
        });
    });

    describe('closeOverlay', () => {
        it('resets activeOverlay to none', () => {
            openSettings();
            closeOverlay();

            expect(get(activeOverlay)).toBe('none');
        });

        it('clears overlayImageUrl', () => {
            openImage('https://example.com/pic.png');
            closeOverlay();

            expect(get(overlayImageUrl)).toBeNull();
        });

        it('is idempotent on already-closed state', () => {
            closeOverlay();

            expect(get(activeOverlay)).toBe('none');
            expect(get(overlayImageUrl)).toBeNull();
        });
    });

    describe('overlay transitions', () => {
        it('opening settings while image is open switches to settings', () => {
            openImage('https://example.com/pic.png');
            openSettings('audio');

            expect(get(activeOverlay)).toBe('settings');
            // Note: overlayImageUrl is NOT cleared by openSettings -- only by closeOverlay
            expect(get(overlayImageUrl)).toBe('https://example.com/pic.png');
        });

        it('opening image while settings is open switches to image', () => {
            openSettings();
            openImage('https://example.com/pic.png');

            expect(get(activeOverlay)).toBe('image');
            expect(get(overlayImageUrl)).toBe('https://example.com/pic.png');
        });

        it('opening connect while settings is open switches to connect', () => {
            openSettings();
            openConnect();

            expect(get(activeOverlay)).toBe('connect');
        });
    });
});
