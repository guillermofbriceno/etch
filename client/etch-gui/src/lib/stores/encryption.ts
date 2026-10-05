import { writable, get } from 'svelte/store';
import type { MatrixCommand, MatrixEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import type { EncryptionStatus } from '$lib/types';
import { registerSessionStore } from './session';

const UNKNOWN: EncryptionStatus = { type: 'Unknown' };

// Core decides the status and sends it on every change and again on every connect.
export const encryptionStatus = writable<EncryptionStatus>(UNKNOWN);
export const encryptionError = writable<string | null>(null);
export const encryptionPromptDismissed = writable(false);
export const encryptionResetOpen = writable(false);
// A request is out and core has answered with neither a status nor a failure.
export const encryptionBusy = writable(false);

registerSessionStore('matrix', 'encryptionStatus', () => { encryptionStatus.set(UNKNOWN); });
registerSessionStore('matrix', 'encryptionError', () => { encryptionError.set(null); });
registerSessionStore('matrix', 'encryptionPromptDismissed', () => { encryptionPromptDismissed.set(false); });
registerSessionStore('matrix', 'encryptionResetOpen', () => { encryptionResetOpen.set(false); });
registerSessionStore('matrix', 'encryptionBusy', () => { encryptionBusy.set(false); });

function sameStatus(a: EncryptionStatus, b: EncryptionStatus): boolean {
    if (a.type === 'RecoveryKeyPending' && b.type === 'RecoveryKeyPending') return a.data.key === b.data.key;
    return a.type === b.type;
}

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

export function showEncryptionPrompt(): void {
    encryptionError.set(null);
    encryptionPromptDismissed.set(false);
}

export function dismissEncryptionPrompt(): void {
    encryptionError.set(null);
    encryptionPromptDismissed.set(true);
}

export function openEncryptionReset(): void {
    encryptionError.set(null);
    encryptionResetOpen.set(true);
}

export function closeEncryptionReset(): void {
    encryptionError.set(null);
    encryptionResetOpen.set(false);
}

export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type === 'EncryptionStatus') {
        // Core reports success only as a status, and a failure always arrives before the status it caused.
        const succeeded = get(encryptionBusy);
        encryptionBusy.set(false);
        // A repeat must not notify, or it would look like a change to whatever is on screen.
        if (!sameStatus(get(encryptionStatus), me.data)) encryptionStatus.set(me.data);
        if (succeeded) {
            encryptionError.set(null);
            encryptionResetOpen.set(false);
            // Whatever core asks for next, such as a new key after a reset, has to be seen.
            encryptionPromptDismissed.set(false);
        }
    } else if (me.type === 'EncryptionActionFailed') {
        encryptionBusy.set(false);
        encryptionError.set(me.data.reason);
    }
}
