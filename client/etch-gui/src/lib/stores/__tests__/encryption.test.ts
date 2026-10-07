import { describe, it, expect, beforeEach } from 'vitest';
import { get } from 'svelte/store';
import { resetStores } from './helpers';
import {
    encryptionError, encryptionBusy, encryptionScreen, encryptionScreenDismissable,
    handleMatrixEvent, resetEncryption, createRecoveryKey,
    openEncryptionDialog, openEncryptionReset, dismissEncryptionScreen,
} from '../encryption';
import { handleMatrixEvent as handleConnectionEvent } from '../matrixConnection';
import { resetMatrixSession } from '../session';
import type { EncryptionStatus } from '$lib/types';

function status(data: EncryptionStatus): void {
    handleMatrixEvent({ type: 'EncryptionStatus', data });
}

function succeeded(): void {
    handleMatrixEvent({ type: 'EncryptionActionSucceeded' });
}

function failed(reason: string): void {
    handleMatrixEvent({ type: 'EncryptionActionFailed', data: { reason } });
}

function connection(type: 'Connecting' | 'Connected'): void {
    handleConnectionEvent({ type: 'ConnectionState', data: { type } });
}

// What a ServerReset does to these stores, followed by the connect that caused it.
function reconnect(data: EncryptionStatus): void {
    resetMatrixSession();
    connection('Connecting');
    connection('Connected');
    status(data);
}

beforeEach(() => {
    resetStores();
    connection('Connected');
});

// That resets clear these stores is covered by their probes in session.test.ts, and what each request sends by EncryptionDialog.test.ts.
describe('the encryption store', () => {
    it('ends a request on its own answer, never on a status that arrives while it is out, and keeps its reason until the next is sent', () => {
        status({ type: 'NeedsRecoveryKey' });
        openEncryptionReset();

        resetEncryption('not the password');
        status({ type: 'NeedsVerifiedDevice' });
        expect(get(encryptionBusy), 'a status says what the device needs, not how a request went').toBe(true);
        expect(get(encryptionScreen)).toBe('reset');

        failed('The password is not correct.');
        expect(get(encryptionBusy)).toBe(false);
        expect(get(encryptionScreen), 'the reason needs the screen it is shown on').toBe('reset');
        expect(get(encryptionError)).toBe('The password is not correct.');

        resetEncryption('password');
        expect(get(encryptionError), 'the reason belonged to the request before this one').toBeNull();
        status({ type: 'NeedsRecoverySetup' });
        succeeded();
        expect(get(encryptionBusy)).toBe(false);
        expect(get(encryptionScreen), 'what the device needs next has to be seen').toBe('create-key');
    });

    it('shows a screen only while there is a session, and keeps it through a sync that degrades', () => {
        resetMatrixSession();
        status({ type: 'NeedsRecoveryKey' });
        expect(get(encryptionScreen), 'its buttons could not work yet').toBeNull();

        connection('Connecting');
        expect(get(encryptionScreen)).toBeNull();
        connection('Connected');
        expect(get(encryptionScreen)).toBe('enter-key');

        connection('Connecting');
        expect(get(encryptionScreen), 'core still accepts the key while it retries the sync').toBe('enter-key');
    });

    it('lets entering a key wait for the rest of the login, and creating a first one only until the next connect', () => {
        status({ type: 'NeedsRecoveryKey' });
        dismissEncryptionScreen();
        expect(get(encryptionScreen)).toBeNull();
        reconnect({ type: 'NeedsRecoveryKey' });
        expect(get(encryptionScreen)).toBeNull();

        status({ type: 'NeedsVerifiedDevice' });
        expect(get(encryptionScreen), 'a different need was never put off').toBe('set-up-device');

        status({ type: 'NeedsRecoveryKey' });
        openEncryptionDialog();
        expect(get(encryptionScreen), 'a control outside the dialog brings it back').toBe('enter-key');

        status({ type: 'NeedsRecoverySetup' });
        createRecoveryKey();
        failed('Could not reach the server.');
        dismissEncryptionScreen();
        expect(get(encryptionScreen)).toBeNull();
        reconnect({ type: 'NeedsRecoverySetup' });
        expect(get(encryptionScreen)).toBe('create-key');
    });

    it('does not let a first key, a key on screen, or a request that is out be walked away from', () => {
        status({ type: 'NeedsRecoverySetup' });
        dismissEncryptionScreen();
        expect(get(encryptionScreen), 'a first key may be skipped only once creating it has failed').toBe('create-key');

        status({ type: 'RecoveryKeyPending', data: { key: 'EsTc abcd' } });
        dismissEncryptionScreen();
        expect(get(encryptionScreen)).toBe('save-key');

        status({ type: 'Ready' });
        openEncryptionDialog();
        expect([get(encryptionScreen), get(encryptionScreenDismissable)]).toEqual(['replace-key', true]);
        createRecoveryKey();
        dismissEncryptionScreen();
        expect(get(encryptionScreen), 'the old key may already be gone').toBe('replace-key');
    });
});
