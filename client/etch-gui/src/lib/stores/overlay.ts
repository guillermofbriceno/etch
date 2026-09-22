import { writable } from 'svelte/store';

export type OverlayType = 'none' | 'settings' | 'image' | 'connect';

// Device-scoped. Which overlay is open, and where it is scrolled to, is the
// user's place in the UI. A reconnect happening behind a settings panel is no
// reason to shut it. overlayImageUrl is the close call: it points at media on
// the server being left, but it is only ever read while activeOverlay is
// 'image', and clearing one without the other leaves a backdrop over nothing.
export const activeOverlay = writable<OverlayType>('none');
export const overlayImageUrl = writable<string | null>(null);
export const settingsTab = writable<string>('voice');
export const showRoomIds = writable<boolean>(false);

export function openSettings(tab: string = 'voice'): void {
    settingsTab.set(tab);
    activeOverlay.set('settings');
}

export function openImage(url: string): void {
    overlayImageUrl.set(url);
    activeOverlay.set('image');
}

export function openConnect(): void {
    activeOverlay.set('connect');
}

export function closeOverlay(): void {
    activeOverlay.set('none');
    overlayImageUrl.set(null);
}
