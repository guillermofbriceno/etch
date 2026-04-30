import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { get } from 'svelte/store';
import { errorLog, toastError, showToast, handleSystemEvent } from '../errors';
import { resetStores } from './helpers';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

beforeEach(() => {
    vi.useFakeTimers();
    resetStores();
});

afterEach(() => {
    vi.useRealTimers();
});

describe('showToast', () => {
    it('sets toastError to the message', () => {
        showToast('Something broke');

        expect(get(toastError)).toBe('Something broke');
    });

    it('auto-clears after 5 seconds', () => {
        showToast('Temporary error');

        expect(get(toastError)).toBe('Temporary error');

        vi.advanceTimersByTime(5000);

        expect(get(toastError)).toBeNull();
    });

    it('does not clear before 5 seconds', () => {
        showToast('Temporary error');

        vi.advanceTimersByTime(4999);

        expect(get(toastError)).toBe('Temporary error');
    });

    it('replaces the previous toast when called again', () => {
        showToast('First error');
        showToast('Second error');

        expect(get(toastError)).toBe('Second error');
    });

    it('resets the timer when a new toast replaces the old one', () => {
        showToast('First error');
        vi.advanceTimersByTime(3000);

        showToast('Second error');
        vi.advanceTimersByTime(3000);

        // 6s total, but only 3s since the second toast -- should still be visible
        expect(get(toastError)).toBe('Second error');

        vi.advanceTimersByTime(2000);

        // 5s since the second toast -- now cleared
        expect(get(toastError)).toBeNull();
    });

    it('first timer does not fire after being replaced', () => {
        showToast('First');
        showToast('Second');

        vi.advanceTimersByTime(5000);

        // After 5s the second toast clears
        expect(get(toastError)).toBeNull();

        // Verify no lingering state from the first timer
        vi.advanceTimersByTime(5000);
        expect(get(toastError)).toBeNull();
    });
});

describe('handleSystemEvent (errors)', () => {
    it('appends to errorLog with message, target, and timestamp', () => {
        const before = Date.now();

        handleSystemEvent({
            type: 'LogError',
            data: { message: 'Connection failed', target: 'etch_core::matrix' },
        } as any);

        const log = get(errorLog);
        expect(log).toHaveLength(1);
        expect(log[0].message).toBe('Connection failed');
        expect(log[0].target).toBe('etch_core::matrix');
        expect(log[0].timestamp.getTime()).toBeGreaterThanOrEqual(before);
    });

    it('accumulates multiple errors in order', () => {
        handleSystemEvent({
            type: 'LogError',
            data: { message: 'Error 1', target: 'a' },
        } as any);
        handleSystemEvent({
            type: 'LogError',
            data: { message: 'Error 2', target: 'b' },
        } as any);
        handleSystemEvent({
            type: 'LogError',
            data: { message: 'Error 3', target: 'c' },
        } as any);

        const log = get(errorLog);
        expect(log).toHaveLength(3);
        expect(log[0].message).toBe('Error 1');
        expect(log[2].message).toBe('Error 3');
    });

    it('triggers showToast with the error message', () => {
        handleSystemEvent({
            type: 'LogError',
            data: { message: 'Visible error', target: 'test' },
        } as any);

        expect(get(toastError)).toBe('Visible error');
    });

    it('rapid errors show only the latest toast', () => {
        handleSystemEvent({
            type: 'LogError',
            data: { message: 'Error A', target: 'test' },
        } as any);
        handleSystemEvent({
            type: 'LogError',
            data: { message: 'Error B', target: 'test' },
        } as any);

        expect(get(toastError)).toBe('Error B');

        // After 5s only the last timer fires
        vi.advanceTimersByTime(5000);
        expect(get(toastError)).toBeNull();
    });

    it('ignores non-LogError events', () => {
        handleSystemEvent({
            type: 'SettingsLoaded',
            data: { bookmarks: [] },
        } as any);

        expect(get(errorLog)).toHaveLength(0);
        expect(get(toastError)).toBeNull();
    });
});
