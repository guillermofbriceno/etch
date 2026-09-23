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
import { resetMatrixSession, declareStores } from './session';

// Store modules classify their state by calling registerSessionStore() or
// declareStores() at import time, so a reset only covers a module something
// has pulled in. Every store module is imported here, by pattern rather than
// by name: a hand-written list of imports would be the reset list's old bug
// wearing a different hat, and a store whose module is only reached through
// some component would otherwise go unregistered until that component was
// first rendered -- possibly after the first ServerReset.
//
// The glob is eager on purpose: the registrations must have run by the time
// initEventRouter() can fire a reset, not at some later await point.
import.meta.glob('./*.ts', { eager: true });

// Device-scoped. Tracks OS window focus for notification sounds, which has
// nothing to do with which server is connected.
export const appFocused = writable(true);

declareStores('device', 'appFocused');

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
