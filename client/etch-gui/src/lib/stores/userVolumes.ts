import { writable } from 'svelte/store';
import { sendCoreCommand } from '$lib/ipc';

export const userVolumes = writable<Record<number, number>>({});

export function resetUserVolumes(): void {
    userVolumes.set({});
}

export function setUserVolume(session_id: number, offset_db: number): void {
    userVolumes.update(v => ({ ...v, [session_id]: offset_db }));
    sendCoreCommand({ type: 'Mumble', data: { type: 'SetUserVolume', data: { session_id, volume_db: offset_db } } });
}
