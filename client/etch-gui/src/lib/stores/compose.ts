import { writable } from 'svelte/store';
import type { ChatMessage } from '$lib/types';

export const replyingTo = writable<ChatMessage | null>(null);

export function setReply(msg: ChatMessage): void {
    replyingTo.set(msg);
}

export function clearReply(): void {
    replyingTo.set(null);
}

export const resetCompose = clearReply;
