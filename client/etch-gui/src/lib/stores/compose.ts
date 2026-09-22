import { writable } from 'svelte/store';
import type { ChatMessage } from '$lib/types';
import { registerSessionStore } from './session';

// Matrix session. Both hold a message from the connected server's timeline;
// replying to or editing one after switching servers would target an event
// that the new session knows nothing about.
export const replyingTo = writable<ChatMessage | null>(null);
export const editingMessage = writable<ChatMessage | null>(null);

registerSessionStore('matrix', 'replyingTo', clearReply);
registerSessionStore('matrix', 'editingMessage', clearEditing);

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
