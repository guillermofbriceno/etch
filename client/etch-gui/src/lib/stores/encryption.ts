import { writable, get } from 'svelte/store';
import type { MatrixCommand, MatrixEvent, SystemEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import type { EncryptionStatus } from '$lib/types';
import { currentUser } from './user';
import { registerSessionStore, declareStores } from './session';

const UNKNOWN: EncryptionStatus = { type: 'Unknown' };

export type PromptDismissal = { account: string; need: EncryptionStatus['type'] };
export type RequestedScreen = 'replace-key' | 'reset';

// Core decides the status and sends it on every change and again on every connect.
export const encryptionStatus = writable<EncryptionStatus>(UNKNOWN);
export const encryptionError = writable<string | null>(null);
// Holds for the rest of the run, but only for the account and the need it was given for.
export const encryptionPromptDismissed = writable<PromptDismissal | null>(null);
// A screen the user asked for, which the status alone would not show.
export const encryptionRequestedScreen = writable<RequestedScreen | null>(null);
// A request is out and core has answered with neither a status nor a failure.
export const encryptionBusy = writable(false);

registerSessionStore('matrix', 'encryptionStatus', () => { encryptionStatus.set(UNKNOWN); });
registerSessionStore('matrix', 'encryptionError', () => { encryptionError.set(null); });
registerSessionStore('matrix', 'encryptionRequestedScreen', () => { encryptionRequestedScreen.set(null); });
registerSessionStore('matrix', 'encryptionBusy', () => { encryptionBusy.set(false); });
declareStores('device', 'encryptionPromptDismissed');

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

/** Opens the screen the current status calls for; every control outside the dialog comes through here. */
export function openEncryptionDialog(): void {
    encryptionError.set(null);
    if (get(encryptionStatus).type === 'Ready') encryptionRequestedScreen.set('replace-key');
    else encryptionPromptDismissed.set(null);
}

export function dismissEncryptionPrompt(): void {
    encryptionError.set(null);
    encryptionPromptDismissed.set({ account: get(currentUser).matrixId, need: get(encryptionStatus).type });
}

export function openEncryptionReset(): void {
    encryptionError.set(null);
    encryptionRequestedScreen.set('reset');
}

export function closeEncryptionScreen(): void {
    encryptionError.set(null);
    encryptionRequestedScreen.set(null);
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
            encryptionRequestedScreen.set(null);
            // Whatever core asks for next, such as a new key after a reset, has to be seen.
            encryptionPromptDismissed.set(null);
        }
    } else if (me.type === 'EncryptionActionFailed') {
        encryptionBusy.set(false);
        encryptionError.set(me.data.reason);
    }
}

export function handleSystemEvent(se: SystemEvent): void {
    const dismissal = get(encryptionPromptDismissed);
    if (dismissal === null) return;
    // The prompt to create a key can only be put off until the next connect, and nothing outlives a sign out.
    const createPromptReturns = se.type === 'ServerReset' && dismissal.need === 'NeedsRecoverySetup';
    if (se.type === 'SignedOut' || createPromptReturns) encryptionPromptDismissed.set(null);
}
