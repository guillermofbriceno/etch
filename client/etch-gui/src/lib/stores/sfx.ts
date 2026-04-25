import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

export type SfxName =
    | 'mute'
    | 'unmute'
    | 'deafen'
    | 'undeafen'
    | 'server_join'
    | 'server_disconnect'
    | 'user_join'
    | 'user_leave'
    | 'new_notif';

/** Volume from 0 to 100. Default 30. */
export const sfxVolume = writable<number>(30);

let deafened = false;

export function setSfxDeafened(d: boolean): void {
    deafened = d;
}

export function playSfx(name: SfxName): void {
    if (deafened) return;
    invoke('play_sfx', { name, volume: get(sfxVolume) / 100 });
}
