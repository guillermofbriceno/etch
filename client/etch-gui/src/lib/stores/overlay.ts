import { writable } from 'svelte/store';
import { declareStores } from './session';

export type OverlayType = 'none' | 'settings' | 'image' | 'connect';

export const activeOverlay = writable<OverlayType>('none');
export const overlayImageUrl = writable<string | null>(null);
export const settingsTab = writable<string>('voice');
export const showRoomIds = writable<boolean>(false);

declareStores('device', 'activeOverlay', 'overlayImageUrl', 'settingsTab', 'showRoomIds');

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
