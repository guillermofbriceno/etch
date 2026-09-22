import { writable } from 'svelte/store';

const COMPACT_KEY = 'compact-chat';

// Device-scoped. A display preference, persisted to localStorage.
export const compactChat = writable<boolean>(localStorage.getItem(COMPACT_KEY) === 'true');

export function initLayout(): void {
    compactChat.subscribe((v) => {
        localStorage.setItem(COMPACT_KEY, String(v));
    });
}
