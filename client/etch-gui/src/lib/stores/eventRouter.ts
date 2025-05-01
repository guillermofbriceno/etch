import { writable, get } from 'svelte/store';
import { listen } from '@tauri-apps/api/event';
import type { CoreEvent } from '$lib/ipc';

import { handleMatrixEvent as messagesHandleMatrix, resetMessages } from './messages';
import { handleMatrixEvent as channelsHandleMatrix, resetChannels } from './channels';
import { handleMatrixEvent as serversHandleMatrix, resetServerConnection } from './servers';
import { handleMatrixEvent as userHandleMatrix, handleSystemEvent as userHandleSystem, resetUser } from './user';
import { handleSystemEvent as serversHandleSystem } from './servers';
import { handleSystemEvent as errorsHandleSystem } from './errors';
import { handleMumbleEvent, handleSystemEvent as voiceHandleSystem } from './voiceState';
import { resetActiveChannel } from './activeChannel';
import { resetCompose } from './compose';
import { resetUserVolumes } from './userVolumes';

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
                if (ce.data.type === 'ServerReset') {
                    resetMessages();
                    resetChannels();
                    resetActiveChannel();
                    resetUser();
                    resetServerConnection();
                    resetCompose();
                    resetUserVolumes();
                }
                serversHandleSystem(ce.data);
                errorsHandleSystem(ce.data);
                voiceHandleSystem(ce.data);
                userHandleSystem(ce.data);
                break;
        }
    });
}
