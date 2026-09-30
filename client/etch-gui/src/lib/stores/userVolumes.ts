import { writable } from 'svelte/store';
import { sendCoreCommand } from '$lib/ipc';
import { registerSessionStore } from './session';

// Voice session: Mumble survives a Matrix reconnect, so clearing this on ServerReset
// would hide live adjustments.
export const userVolumes = writable<Record<string, number>>({});

export function resetUserVolumes(): void {
    userVolumes.set({});
}

registerSessionStore('voice', 'userVolumes', resetUserVolumes);

export function setUserVolume(username: string, session_id: number, offset_db: number): void {
    userVolumes.update(v => ({ ...v, [username]: offset_db }));
    sendCoreCommand({ type: 'Mumble', data: { type: 'SetUserVolume', data: { session_id, volume_db: offset_db } } });
}
