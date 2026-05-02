import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { get } from 'svelte/store';

describe('layout store', () => {
    beforeEach(() => {
        localStorage.clear();
        vi.resetModules();
    });

    afterEach(() => {
        localStorage.clear();
    });

    it('defaults to false when localStorage is empty', async () => {
        const { compactChat } = await import('../layout');
        expect(get(compactChat)).toBe(false);
    });

    it('initializes to true when localStorage has "true"', async () => {
        localStorage.setItem('compact-chat', 'true');
        const { compactChat } = await import('../layout');
        expect(get(compactChat)).toBe(true);
    });

    it('initializes to false for any non-"true" localStorage value', async () => {
        localStorage.setItem('compact-chat', 'false');
        const { compactChat } = await import('../layout');
        expect(get(compactChat)).toBe(false);
    });

    it('initLayout subscribes so .set() auto-persists to localStorage', async () => {
        const { compactChat, initLayout } = await import('../layout');
        initLayout();

        compactChat.set(true);
        expect(localStorage.getItem('compact-chat')).toBe('true');

        compactChat.set(false);
        expect(localStorage.getItem('compact-chat')).toBe('false');
    });

    it('without initLayout, .set() does not persist', async () => {
        const { compactChat } = await import('../layout');

        compactChat.set(true);
        expect(localStorage.getItem('compact-chat')).toBeNull();
    });
});
