import { writable } from 'svelte/store';

// The currently selected channel ID.
// Shared leaf dependency for both messages.ts and channels.ts,
// breaking what was previously a circular import.
export const activeChannelId = writable<string | null>(null);

export function resetActiveChannel(): void {
    activeChannelId.set(null);
}

// Simple pub/sub for unread message notifications.
// messages.ts publishes via emitUnreadMessage(), channels.ts subscribes via onUnreadMessage().
type UnreadListener = (roomId: string) => void;
let _unreadListener: UnreadListener | null = null;

export function onUnreadMessage(fn: UnreadListener): void {
    _unreadListener = fn;
}

export function emitUnreadMessage(roomId: string): void {
    _unreadListener?.(roomId);
}
