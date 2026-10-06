import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { resetStores } from './helpers';
import {
    encryptionStatus, encryptionError, encryptionPromptDismissed, encryptionRequestedScreen, encryptionBusy,
    handleMatrixEvent, handleSystemEvent, submitRecoveryKey, resetEncryption, dismissEncryptionPrompt, openEncryptionReset,
} from '../encryption';
import type { EncryptionStatus } from '$lib/types';

function status(data: EncryptionStatus): void {
    handleMatrixEvent({ type: 'EncryptionStatus', data });
}

function failed(reason: string): void {
    handleMatrixEvent({ type: 'EncryptionActionFailed', data: { reason } });
}

beforeEach(() => {
    resetStores();
    vi.mocked(invoke).mockClear();
});

// That a Matrix session reset clears these stores is covered by their probes in session.test.ts.
describe('the encryption store', () => {
    it('follows the status core reports, and does not notify for a repeat', () => {
        const seen: EncryptionStatus[] = [];
        const stop = encryptionStatus.subscribe((s) => seen.push(s));

        status({ type: 'NeedsRecoveryKey' });
        status({ type: 'NeedsRecoveryKey' });
        status({ type: 'RecoveryKeyPending', data: { key: 'EsTc abcd' } });
        status({ type: 'RecoveryKeyPending', data: { key: 'EsTc abcd' } });
        status({ type: 'RecoveryKeyPending', data: { key: 'EsTc wxyz' } });
        stop();

        expect(seen).toEqual([
            { type: 'Unknown' },
            { type: 'NeedsRecoveryKey' },
            { type: 'RecoveryKeyPending', data: { key: 'EsTc abcd' } },
            { type: 'RecoveryKeyPending', data: { key: 'EsTc wxyz' } },
        ]);
    });

    it('keeps the reason a request failed until the next request is sent', () => {
        status({ type: 'NeedsRecoveryKey' });

        submitRecoveryKey('EsTc abcd');
        expect(get(encryptionBusy)).toBe(true);
        failed('That recovery key is not correct.');

        expect(get(encryptionBusy)).toBe(false);
        expect(get(encryptionError)).toBe('That recovery key is not correct.');
        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: { type: 'Matrix', data: { type: 'SubmitRecoveryKey', data: { key: 'EsTc abcd' } } },
        });

        submitRecoveryKey('EsTc wxyz');
        expect(get(encryptionError)).toBeNull();
    });

    it('takes the first status after a request as its success', () => {
        status({ type: 'NeedsRecoveryKey' });
        dismissEncryptionPrompt();
        openEncryptionReset();

        resetEncryption('password');
        status({ type: 'NeedsRecoverySetup' });

        expect(get(encryptionBusy)).toBe(false);
        expect(get(encryptionRequestedScreen)).toBeNull();
        expect(get(encryptionPromptDismissed), 'the prompt for the new key must not stay hidden').toBeNull();
    });

    it('leaves the reset view open after a failed reset, whatever status follows', () => {
        status({ type: 'NeedsRecoveryKey' });
        openEncryptionReset();

        resetEncryption('not the password');
        failed('The password is not correct.');
        status({ type: 'NeedsVerifiedDevice' });

        expect(get(encryptionRequestedScreen)).toBe('reset');
        expect(get(encryptionError)).toBe('The password is not correct.');
    });

    it('lets the prompt to create a key be put off only until the next connect', () => {
        status({ type: 'NeedsRecoveryKey' });
        dismissEncryptionPrompt();
        handleSystemEvent({ type: 'ServerReset' });
        expect(get(encryptionPromptDismissed), 'entering a key can wait for the rest of the run').not.toBeNull();

        status({ type: 'NeedsRecoverySetup' });
        dismissEncryptionPrompt();
        handleSystemEvent({ type: 'ServerReset' });
        expect(get(encryptionPromptDismissed), 'a first key is asked for again on every connect').toBeNull();
    });
});
