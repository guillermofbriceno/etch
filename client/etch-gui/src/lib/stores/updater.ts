import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { relaunch } from '@tauri-apps/plugin-process';

export type UpdateStatus = 'idle' | 'checking' | 'available' | 'ready' | 'up_to_date' | 'error';

export const updateStatus = writable<UpdateStatus>('idle');
export const updateVersion = writable<string | null>(null);
export const updateError = writable<string | null>(null);

listen<{ type: string; data?: any }>('update_event', (event) => {
    const payload = event.payload;
    switch (payload.type) {
        case 'Available':
            updateVersion.set(payload.data.version);
            updateStatus.set('available');
            break;
        case 'Ready':
            updateStatus.set('ready');
            break;
        case 'UpToDate':
            updateStatus.set('up_to_date');
            break;
    }
});

export async function checkForUpdate(): Promise<void> {
    updateStatus.set('checking');
    updateError.set(null);
    try {
        await invoke('check_for_update');
    } catch (e) {
        updateError.set(String(e));
        updateStatus.set('error');
    }
}

export async function restartApp(): Promise<void> {
    await relaunch();
}
