import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { tick } from 'svelte';
import { invoke } from '@tauri-apps/api/core';
import { resetStores } from '$lib/stores/__tests__/helpers';
import { matrixConnected } from '$lib/stores/servers';
import { handleMatrixEvent } from '$lib/stores/encryption';
import { resetMatrixSession } from '$lib/stores/session';
import type { CoreCommand, MatrixCommand } from '$lib/ipc';
import type { EncryptionStatus } from '$lib/types';
import EncryptionDialog from '../EncryptionDialog.svelte';

const KEY = 'EsTc abcd efgh';

async function report(data: EncryptionStatus): Promise<void> {
    handleMatrixEvent({ type: 'EncryptionStatus', data });
    await tick();
}

async function fail(reason: string): Promise<void> {
    handleMatrixEvent({ type: 'EncryptionActionFailed', data: { reason } });
    await tick();
}

function sent(type: MatrixCommand['type']): CoreCommand[] {
    return vi.mocked(invoke).mock.calls
        .map(([, args]) => (args as { command: CoreCommand }).command)
        .filter((command) => command.type === 'Matrix' && command.data.type === type);
}

function heading(): string | null {
    return screen.queryByRole('heading')?.textContent ?? null;
}

beforeEach(() => {
    resetStores();
    vi.mocked(invoke).mockClear();
    matrixConnected.set(true);
});

describe('EncryptionDialog', () => {
    it.each<[EncryptionStatus, string | null]>([
        [{ type: 'Unknown' }, null],
        [{ type: 'Ready' }, null],
        [{ type: 'NeedsRecoverySetup' }, 'Save your recovery key'],
        [{ type: 'RecoveryKeyPending', data: { key: KEY } }, 'Your recovery key'],
        [{ type: 'NeedsRecoveryKey' }, 'Enter your recovery key'],
        [{ type: 'NeedsVerifiedDevice' }, 'Encryption is not set up on this device'],
    ])('shows the screen for %j', async (status, title) => {
        render(EncryptionDialog);

        await report(status);

        expect(heading()).toBe(title);
        if (status.type === 'RecoveryKeyPending') expect(screen.getByText(KEY)).toBeInTheDocument();
    });

    it('waits for the connection, and keeps a half-typed key through a reconnect', async () => {
        const user = userEvent.setup();
        matrixConnected.set(false);
        render(EncryptionDialog);

        await report({ type: 'NeedsRecoveryKey' });
        expect(heading(), 'its button could not work yet').toBeNull();

        matrixConnected.set(true);
        await tick();
        await user.type(screen.getByRole('textbox', { name: 'Recovery key' }), 'EsTc ab');

        resetMatrixSession();
        await tick();
        expect(heading()).toBeNull();
        matrixConnected.set(true);
        await report({ type: 'NeedsRecoveryKey' });

        expect(screen.getByRole('textbox', { name: 'Recovery key' })).toHaveValue('EsTc ab');
    });

    it('sends the key, says why it was refused, and can be put off', async () => {
        const user = userEvent.setup();
        render(EncryptionDialog);
        await report({ type: 'NeedsRecoveryKey' });

        await user.type(screen.getByRole('textbox', { name: 'Recovery key' }), ` ${KEY} `);
        await user.click(screen.getByRole('button', { name: 'Continue' }));

        expect(sent('SubmitRecoveryKey')).toEqual([
            { type: 'Matrix', data: { type: 'SubmitRecoveryKey', data: { key: KEY } } },
        ]);

        await fail('That recovery key is not correct.');
        expect(screen.getByRole('alert')).toHaveTextContent('That recovery key is not correct.');

        await user.click(screen.getByRole('button', { name: 'Not now' }));
        expect(heading()).toBeNull();
    });

    it('offers no way past a new key except confirming it was saved', async () => {
        const user = userEvent.setup();
        render(EncryptionDialog);
        await report({ type: 'RecoveryKeyPending', data: { key: KEY } });

        await user.keyboard('{Escape}');

        expect(heading()).toBe('Your recovery key');
        expect(screen.queryByRole('button', { name: 'Not now' })).not.toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Close dialog' })).not.toBeInTheDocument();

        await user.click(screen.getByRole('button', { name: 'I have saved it' }));
        expect(sent('ConfirmRecoveryKeySaved')).toHaveLength(1);
    });

    it('resets only on its own button, once a password is typed', async () => {
        const user = userEvent.setup();
        render(EncryptionDialog);
        await report({ type: 'NeedsRecoveryKey' });

        await user.click(screen.getByRole('button', { name: "I don't have my key" }));
        await user.click(screen.getByRole('button', { name: 'Reset encryption' }));

        expect(heading()).toBe('Reset encryption');
        const reset = screen.getByRole('button', { name: 'Reset encryption' });
        expect(reset).toBeDisabled();

        await user.type(screen.getByLabelText('Account password'), 'password{Enter}');
        expect(sent('ResetEncryption'), 'Enter must not start something that cannot be undone').toEqual([]);

        await user.click(reset);
        expect(sent('ResetEncryption')).toEqual([
            { type: 'Matrix', data: { type: 'ResetEncryption', data: { password: 'password' } } },
        ]);
    });
});
