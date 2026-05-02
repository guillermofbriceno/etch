import { writable, derived, get } from 'svelte/store';
import { appFocused } from './eventRouter';

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
export const sidebarPeeking = writable<boolean>(false);

// True when the sidebar should render its collapsed (centered) layout.
// False when expanded OR peeking (both show expanded layout).
export const sidebarVisuallyCollapsed = derived(
    [sidebarCollapsed, sidebarPeeking],
    ([$collapsed, $peeking]) => $collapsed && !$peeking
);

// Content state: controls text vs first-letter, full names vs icons, etc.
// Instant swap on peek. Delayed swap on toggle (syncs with grid animation).
export const sidebarContentCollapsed = writable<boolean>(loadCollapsed());
export const sidebarTransitioning = writable<boolean>(false);
let contentTimer: ReturnType<typeof setTimeout> | undefined;

function clearContentTimer(): void {
    if (contentTimer !== undefined) {
        clearTimeout(contentTimer);
        contentTimer = undefined;
    }
}

let peekCloseTimer: ReturnType<typeof setTimeout> | undefined;

function clearPeekCloseTimer(): void {
    if (peekCloseTimer !== undefined) {
        clearTimeout(peekCloseTimer);
        peekCloseTimer = undefined;
    }
}

/** Set peeking state. Only has an effect when the sidebar is collapsed. */
export function setPeeking(peeking: boolean): void {
    if (!get(sidebarCollapsed)) return;
    clearContentTimer();
    clearPeekCloseTimer();
    sidebarTransitioning.set(false);
    sidebarPeeking.set(peeking);
    sidebarContentCollapsed.set(!peeking);
}

/** Begin closing the peek after a short delay. Call cancelPeekClose to abort. */
export function startPeekClose(delayMs = 150): void {
    if (!get(sidebarPeeking)) return;
    clearPeekCloseTimer();
    sidebarTransitioning.set(true);
    peekCloseTimer = setTimeout(() => {
        peekCloseTimer = undefined;
        setPeeking(false);
    }, delayMs);
}

/** Cancel a pending peek close (e.g. mouse re-entered the sidebar). */
export function cancelPeekClose(): void {
    if (peekCloseTimer === undefined) return;
    clearPeekCloseTimer();
    sidebarTransitioning.set(false);
}

export function toggleSidebar(): void {
    const collapsed = !get(sidebarCollapsed);
    try {
        localStorage.setItem(STORAGE_KEY, String(collapsed));
    } catch { /* ignore */ }

    clearContentTimer();
    sidebarPeeking.set(false);

    // If content is already in the target state (e.g. toggling while peeking),
    // skip the transition.
    if (get(sidebarContentCollapsed) === collapsed) {
        sidebarCollapsed.set(collapsed);
        sidebarTransitioning.set(false);
        return;
    }

    sidebarTransitioning.set(true);
    sidebarCollapsed.set(collapsed);

    const delay = collapsed ? FADE_MS : WIDTH_TRANSITION_MS;
    contentTimer = setTimeout(() => {
        contentTimer = undefined;
        sidebarContentCollapsed.set(collapsed);
        sidebarTransitioning.set(false);
    }, delay);
}

// Collapse peek when the app window loses focus.
const unsubFocused = appFocused.subscribe(focused => {
    if (!focused && get(sidebarPeeking)) {
        clearContentTimer();
        sidebarPeeking.set(false);
        sidebarContentCollapsed.set(true);
        sidebarTransitioning.set(false);
    }
});

/** Clean up timers and subscriptions. Call in tests or when the module is no longer needed. */
export function destroySidebar(): void {
    clearContentTimer();
    clearPeekCloseTimer();
    unsubFocused();
}
