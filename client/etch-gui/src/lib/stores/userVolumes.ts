import { writable } from 'svelte/store';
import { sendCoreCommand } from '$lib/ipc';
import { registerSessionStore } from './session';

// Voice session. Keyed by Mumble username, which is only meaningful on the
// voice server we are attached to. This hangs off the voice lifecycle rather
// than ServerReset because Mumble survives a Matrix reconnect: clearing the
// map there would leave every slider reading 0 dB while Mumble carried on
// applying the adjustments the user cannot see any more.
export const userVolumes = writable<Record<string, number>>({});

export function resetUserVolumes(): void {
    userVolumes.set({});
}

registerSessionStore('voice', 'userVolumes', resetUserVolumes);

export function setUserVolume(username: string, session_id: number, offset_db: number): void {
    userVolumes.update(v => ({ ...v, [username]: offset_db }));
    sendCoreCommand({ type: 'Mumble', data: { type: 'SetUserVolume', data: { session_id, volume_db: offset_db } } });
}
