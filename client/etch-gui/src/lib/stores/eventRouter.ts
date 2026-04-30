import { writable, get } from 'svelte/store';
import { listen } from '@tauri-apps/api/event';
import type { CoreEvent } from '$lib/ipc';

import { handleMatrixEvent as messagesHandleMatrix } from './messages';
import { handleMatrixEvent as channelsHandleMatrix } from './channels';
import { handleMatrixEvent as serversHandleMatrix } from './servers';
import { handleMatrixEvent as userHandleMatrix, handleSystemEvent as userHandleSystem } from './user';
import { handleSystemEvent as serversHandleSystem } from './servers';
import { handleSystemEvent as errorsHandleSystem } from './errors';
import { handleMumbleEvent, handleSystemEvent as voiceHandleSystem } from './voiceState';

// Track app focus state for notification sounds
export const appFocused = writable(true);

export function initEventRouter(): void {
    appFocused.set(document.hasFocus());
    listen('tauri://focus', () => { appFocused.set(true); });
    listen('tauri://blur', () => { appFocused.set(false); });

    listen<CoreEvent>('core_event', (event) => {
        const ce = event.payload;

        switch (ce.type) {
            case 'Matrix':
                messagesHandleMatrix(ce.data);
                channelsHandleMatrix(ce.data);
                serversHandleMatrix(ce.data);
                userHandleMatrix(ce.data);
                break;
            case 'Mumble':
                handleMumbleEvent(ce.data);
                break;
            case 'System':
                serversHandleSystem(ce.data);
                errorsHandleSystem(ce.data);
                voiceHandleSystem(ce.data);
                userHandleSystem(ce.data);
                break;
        }
    });
}
