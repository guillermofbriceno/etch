import { writable, get } from 'svelte/store';
import { sendCoreCommand } from '$lib/ipc';
import { playSfx, setSfxDeafened } from './sfx';
import { declareStores } from './session';

// Backend-owned: the engine restores them across Mumble restarts, so a reset must not clear them.
export const isMuted = writable<boolean>(false);
export const isDeafened = writable<boolean>(false);

declareStores('backend', 'isMuted', 'isDeafened');

export async function toggleMute(): Promise<void> {
    if (get(isDeafened)) {
        // Unmuting while deafened should undeafen entirely
        isDeafened.set(false);
        isMuted.set(false);
        setSfxDeafened(false);
        playSfx('undeafen');
        await sendCoreCommand({ type: 'System', data: { type: 'Deafen', data: false } });
        return;
    }
    const next = !get(isMuted);
    isMuted.set(next);
    playSfx(next ? 'mute' : 'unmute');
    await sendCoreCommand({ type: 'System', data: { type: 'MuteMic', data: next } });
}

export async function toggleDeafen(): Promise<void> {
    const next = !get(isDeafened);
    isDeafened.set(next);
    if (!next) setSfxDeafened(false);
    playSfx(next ? 'deafen' : 'undeafen');
    if (next) setSfxDeafened(true);
    await sendCoreCommand({ type: 'System', data: { type: 'Deafen', data: next } });
    // Mumble handles mute state automatically with deafen/undeafen
    isMuted.set(next);
}
