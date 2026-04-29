import { writable } from 'svelte/store';
import type { SystemEvent } from '$lib/ipc';

export type ErrorEntry = {
    message: string;
    target: string;
    timestamp: Date;
};

export const errorLog = writable<ErrorEntry[]>([]);
export const toastError = writable<string | null>(null);

let toastTimer: ReturnType<typeof setTimeout> | null = null;

export function showToast(message: string) {
    if (toastTimer) clearTimeout(toastTimer);
    toastError.set(message);
    toastTimer = setTimeout(() => {
        toastError.set(null);
        toastTimer = null;
    }, 5000);
}

// Handler called by eventRouter for System error events
export function handleSystemEvent(se: SystemEvent): void {
    if (se.type !== 'LogError') return;

    errorLog.update(log => [
        ...log,
        {
            message: se.data.message,
            target: se.data.target,
            timestamp: new Date(),
        },
    ]);

    showToast(se.data.message);
}
