import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import type { ChatMessage, RoomInfo } from '$lib/types';
import { appFocused } from './eventRouter';
import { isDeafened } from './audio';
import { deafenSuppressesNotifs } from './voiceSettings';
import { declareStores } from './session';

const ENABLED_KEY = 'desktop-notifications';

export const desktopNotifications = writable<boolean>(localStorage.getItem(ENABLED_KEY) !== 'false');

declareStores('device', 'desktopNotifications');

export function setDesktopNotifications(value: boolean): void {
    desktopNotifications.set(value);
    localStorage.setItem(ENABLED_KEY, String(value));
}

// Deafen silences these as it does the sounds, messages included unless the user exempted them.
function show(kind: 'message' | 'voice', title: string, body: string): void {
    if (!get(desktopNotifications) || get(appFocused)) return;
    if (get(isDeafened) && (kind === 'voice' || get(deafenSuppressesNotifs))) return;
    invoke('show_notification', { title, body });
}

function roomLabel(room: RoomInfo): string {
    return room.etch_room_type === 'Text' ? `#${room.display_name}` : room.display_name;
}

export function notifyMessage(room: RoomInfo | undefined, senderName: string, message: ChatMessage): void {
    const title = !room || room.etch_room_type === 'Dm' ? senderName : `${senderName} in ${roomLabel(room)}`;
    show('message', title, message.body);
}

export function notifyVoicePresence(userName: string, joined: boolean, channelName: string | undefined): void {
    show('voice', userName, `${joined ? 'Joined' : 'Left'} ${channelName ?? 'your voice channel'}`);
}
