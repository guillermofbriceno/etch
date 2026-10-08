import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { get } from 'svelte/store';

describe('followNewMessages', () => {
    beforeEach(() => {
        localStorage.clear();
        vi.resetModules();
    });

    afterEach(() => {
        localStorage.clear();
    });

    it('is off until turned on, and a later start remembers the choice', async () => {
        const first = await import('../followMessages');
        expect(get(first.followNewMessages)).toBe(false);

        first.setFollowNewMessages(true);
        vi.resetModules();
        const second = await import('../followMessages');

        expect(get(second.followNewMessages)).toBe(true);
    });
});
