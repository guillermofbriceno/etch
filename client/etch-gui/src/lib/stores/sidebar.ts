import { writable, get } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

const STORAGE_KEY = 'sidebar-collapsed';
const FADE_MS = 200;
const WIDTH_TRANSITION_MS = 100;

function loadCollapsed(): boolean {
    try {
        return localStorage.getItem(STORAGE_KEY) === 'true';
    } catch {
        return false;
    }
}

export const sidebarCollapsed = writable<boolean>(loadCollapsed());

// Temporary flag: forces text opacity to 0 during toggle animation.
// Masks the container-query content swap so it happens invisibly.
export const sidebarTransitioning = writable<boolean>(false);
let contentTimer: ReturnType<typeof setTimeout> | undefined;

function clearContentTimer(): void {
    if (contentTimer !== undefined) {
        clearTimeout(contentTimer);
        contentTimer = undefined;
    }
}

export function toggleSidebar(): void {
    const collapsed = !get(sidebarCollapsed);
    try {
        localStorage.setItem(STORAGE_KEY, String(collapsed));
    } catch { /* ignore */ }

    clearContentTimer();
    sidebarTransitioning.set(true);
    sidebarCollapsed.set(collapsed);

    const delay = collapsed ? FADE_MS : WIDTH_TRANSITION_MS;
    contentTimer = setTimeout(() => {
        contentTimer = undefined;
        sidebarTransitioning.set(false);
    }, delay);
}

/** Whether the sidebar peek (hover-expand) should be suppressed because the
 *  cursor has left the window. Only relevant on Linux where GTK cursor events
 *  drive this state. */
export const peekSuppressed = writable<boolean>(false);

let unlistenLeave: UnlistenFn | undefined;
let unlistenEnter: UnlistenFn | undefined;

export async function initCursorTracking(): Promise<void> {
    unlistenLeave = await listen('cursor-left-window', () => {
        peekSuppressed.set(true);
    });
    unlistenEnter = await listen('cursor-entered-window', () => {
        peekSuppressed.set(false);
    });
}

function destroyCursorTracking(): void {
    unlistenLeave?.();
    unlistenEnter?.();
    unlistenLeave = undefined;
    unlistenEnter = undefined;
}

/** Clean up timers and event listeners. */
export function destroySidebar(): void {
    clearContentTimer();
    destroyCursorTracking();
}
