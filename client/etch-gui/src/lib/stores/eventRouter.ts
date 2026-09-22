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
import { resetMatrixSession } from './session';

// Store modules declare their session-scoped state by calling
// registerSessionStore() at import time, so a reset only knows about a module
// something has pulled in. The handler imports above cover most of them; these
// three own session state but no event handler, so they need an import of
// their own to get registered.
import './activeChannel';
import './compose';
import './userVolumes';

// Device-scoped. Tracks OS window focus for notification sounds, which has
// nothing to do with which server is connected.
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
                    // Matrix only. The voice session has its own lifecycle and
                    // survives a homeserver reconnect; voiceState.ts clears it
                    // when Mumble actually disconnects.
                    resetMatrixSession();
                }
                serversHandleSystem(ce.data);
                errorsHandleSystem(ce.data);
                voiceHandleSystem(ce.data);
                userHandleSystem(ce.data);
                break;
        }
    });
}
