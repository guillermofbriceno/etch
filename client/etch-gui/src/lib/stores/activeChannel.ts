import { writable } from 'svelte/store';
import { registerSessionStore } from './session';

// The currently selected channel ID.
// Shared leaf dependency for both messages.ts and channels.ts,
// breaking what was previously a circular import.
export const activeChannelId = writable<string | null>(null);

export function resetActiveChannel(): void {
    activeChannelId.set(null);
}

registerSessionStore('matrix', 'activeChannelId', resetActiveChannel);
