import { writable, derived, get } from 'svelte/store';
import type { MatrixCommand, MatrixEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import type { EncryptionStatus } from '$lib/types';
import { matrixSessionLive } from './matrixConnection';
import { registerSessionStore, declareStores } from './session';

type Status = EncryptionStatus['type'];

export type EncryptionScreen = 'create-key' | 'save-key' | 'enter-key' | 'set-up-device' | 'replace-key' | 'reset';
/** A screen that some control opens. The key screen opens itself and cannot be left for later. */
export type EntryScreen = Exclude<EncryptionScreen, 'save-key'>;
/** A screen through which a device that cannot read encrypted messages yet comes to read them. */
export type UnlockScreen = 'enter-key' | 'set-up-device';

const UNKNOWN: EncryptionStatus = { type: 'Unknown' };

// Core decides the status and sends it on every change and again on every connect.
export const encryptionStatus = writable<EncryptionStatus>(UNKNOWN);
export const encryptionError = writable<string | null>(null);
// A request is out and core has not answered it yet.
export const encryptionBusy = writable(false);
// A screen the user asked for, which the status alone would not show.
export const encryptionRequestedScreen = writable<'replace-key' | 'reset' | null>(null);
// The need the user chose "Not Now" for.
export const encryptionPromptDismissed = writable<Status | null>(null);
// Creating a first key is not something to skip for long, so this lasts only until the next connect.
export const createKeyPromptPutOff = writable(false);

registerSessionStore('matrix', 'encryptionStatus', () => { encryptionStatus.set(UNKNOWN); });
registerSessionStore('matrix', 'encryptionError', () => { encryptionError.set(null); });
registerSessionStore('matrix', 'encryptionBusy', () => { encryptionBusy.set(false); });
registerSessionStore('matrix', 'encryptionRequestedScreen', () => { encryptionRequestedScreen.set(null); });
registerSessionStore('matrix', 'createKeyPromptPutOff', () => { createKeyPromptPutOff.set(false); });
registerSessionStore('login', 'encryptionPromptDismissed', () => { encryptionPromptDismissed.set(null); });

function promptFor(status: Status): EncryptionScreen | null {
    switch (status) {
        case 'NeedsRecoverySetup': return 'create-key';
        case 'RecoveryKeyPending': return 'save-key';
        case 'NeedsRecoveryKey': return 'enter-key';
        case 'NeedsVerifiedDevice': return 'set-up-device';
        default: return null;
    }
}

export function entryScreen(status: Status): EntryScreen | null {
    if (status === 'Ready') return 'replace-key';
    const prompt = promptFor(status);
    return prompt === 'save-key' ? null : prompt;
}

export function unlockScreen(status: Status): UnlockScreen | null {
    const prompt = promptFor(status);
    return prompt === 'enter-key' || prompt === 'set-up-device' ? prompt : null;
}

/** Until a key is saved, this device holds the only copy of what unlocks the encrypted messages. */
export function hasNoSavedKey(status: Status): boolean {
    return status === 'NeedsRecoverySetup' || status === 'RecoveryKeyPending';
}

export const encryptionScreen = derived(
    [encryptionStatus, matrixSessionLive, encryptionRequestedScreen, encryptionPromptDismissed, createKeyPromptPutOff],
    ([$status, $live, $requested, $dismissed, $putOff]): EncryptionScreen | null => {
        if (!$live) return null;
        const status = $status.type;
        if (status === 'RecoveryKeyPending') return 'save-key';
        if ($requested === 'reset') return 'reset';
        if ($requested === 'replace-key' && status === 'Ready') return 'replace-key';
        const skipped = status === 'NeedsRecoverySetup' ? $putOff : $dismissed === status;
        return skipped ? null : promptFor(status);
    },
);

export const encryptionScreenDismissable = derived(
    [encryptionScreen, encryptionBusy, encryptionError],
    ([$screen, $busy, $error]): boolean => {
        switch ($screen) {
            case 'enter-key':
            case 'set-up-device':
                return true;
            // A first key may be skipped only once creating it has failed.
            case 'create-key':
                return $error !== null;
            case 'replace-key':
            case 'reset':
                return !$busy;
            default:
                return false;
        }
    },
);

declareStores('derived', 'encryptionScreen', 'encryptionScreenDismissable');

function request(command: MatrixCommand): void {
    encryptionError.set(null);
    encryptionBusy.set(true);
    sendCoreCommand({ type: 'Matrix', data: command }).catch((e) => {
        encryptionBusy.set(false);
        encryptionError.set(`Etch could not send the request: ${e}`);
    });
}

export function createRecoveryKey(): void {
    request({ type: 'CreateRecoveryKey' });
}

export function confirmRecoveryKeySaved(): void {
    request({ type: 'ConfirmRecoveryKeySaved' });
}

export function submitRecoveryKey(key: string): void {
    request({ type: 'SubmitRecoveryKey', data: { key } });
}

export function resetEncryption(password: string): void {
    request({ type: 'ResetEncryption', data: { password } });
}

/** Opens the screen the current status has an entry for; every control outside the dialog comes through here. */
export function openEncryptionDialog(): void {
    encryptionError.set(null);
    if (get(encryptionStatus).type === 'Ready') {
        encryptionRequestedScreen.set('replace-key');
    } else {
        encryptionPromptDismissed.set(null);
        createKeyPromptPutOff.set(false);
    }
}

export function openEncryptionReset(): void {
    encryptionError.set(null);
    encryptionRequestedScreen.set('reset');
}

/** Leaves the screen that is showing, where that is allowed. */
export function dismissEncryptionScreen(): void {
    if (!get(encryptionScreenDismissable)) return;
    const screen = get(encryptionScreen);
    encryptionError.set(null);
    if (screen === 'replace-key' || screen === 'reset') encryptionRequestedScreen.set(null);
    else if (screen === 'create-key') createKeyPromptPutOff.set(true);
    else encryptionPromptDismissed.set(get(encryptionStatus).type);
}

export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type === 'EncryptionStatus') {
        encryptionStatus.set(me.data);
    } else if (me.type === 'EncryptionActionSucceeded') {
        encryptionBusy.set(false);
        encryptionError.set(null);
        encryptionRequestedScreen.set(null);
        // Whatever the device needs next, such as a new key after a reset, has to be seen.
        encryptionPromptDismissed.set(null);
        createKeyPromptPutOff.set(false);
    } else if (me.type === 'EncryptionActionFailed') {
        encryptionBusy.set(false);
        encryptionError.set(me.data.reason);
    }
}
