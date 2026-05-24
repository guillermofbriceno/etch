import { writable } from 'svelte/store';
import type { ChatMessage } from '$lib/types';

export const replyingTo = writable<ChatMessage | null>(null);
export const editingMessage = writable<ChatMessage | null>(null);

export function setReply(msg: ChatMessage): void {
    editingMessage.set(null);
    replyingTo.set(msg);
}

export function clearReply(): void {
    replyingTo.set(null);
}

export function setEditing(msg: ChatMessage): void {
    replyingTo.set(null);
    editingMessage.set(msg);
}

export function clearEditing(): void {
    editingMessage.set(null);
}

export function resetCompose(): void {
    clearReply();
    clearEditing();
}
