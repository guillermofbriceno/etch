import { writable } from 'svelte/store';
import type { ChatMessage } from '$lib/types';
import { registerSessionStore } from './session';

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

registerSessionStore('matrix', 'replyingTo', clearReply);
registerSessionStore('matrix', 'editingMessage', clearEditing);
