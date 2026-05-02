import { writable, get } from 'svelte/store';

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

/** Clean up timers. Call in tests or when the module is no longer needed. */
export function destroySidebar(): void {
    clearContentTimer();
}
