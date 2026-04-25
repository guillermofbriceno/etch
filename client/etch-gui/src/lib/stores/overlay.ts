import { writable } from 'svelte/store';

export type OverlayType = 'none' | 'settings' | 'image' | 'connect';

export const activeOverlay = writable<OverlayType>('none');
export const overlayImageUrl = writable<string | null>(null);
export const settingsTab = writable<string>('voice');

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
