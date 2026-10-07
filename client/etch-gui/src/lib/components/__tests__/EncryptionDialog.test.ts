import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { tick } from 'svelte';
import { invoke } from '@tauri-apps/api/core';
import { resetStores } from '$lib/stores/__tests__/helpers';
import { matrixStatus } from '$lib/stores/matrixConnection';
import { handleMatrixEvent } from '$lib/stores/encryption';
import { resetMatrixSession } from '$lib/stores/session';
import type { CoreCommand, MatrixCommand } from '$lib/ipc';
import type { EncryptionStatus } from '$lib/types';
import EncryptionDialog from '../EncryptionDialog.svelte';
import SettingsAccount from '../settings/SettingsAccount.svelte';

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
    matrixStatus.set('connected');
});

// Which screen shows when, and what may be put off for how long, is the store's to decide and is covered in encryption.test.ts.
describe('EncryptionDialog', () => {
    it.each<[EncryptionStatus, string | null]>([
        [{ type: 'Unknown' }, null],
        [{ type: 'Ready' }, null],
        [{ type: 'NeedsRecoverySetup' }, 'Create Your Recovery Key'],
        [{ type: 'RecoveryKeyPending', data: { key: KEY } }, 'Save Your Recovery Key'],
        [{ type: 'NeedsRecoveryKey' }, 'Enter Your Recovery Key'],
        [{ type: 'NeedsVerifiedDevice' }, 'Set Up This Device'],
    ])('shows the screen for %j', async (status, title) => {
        render(EncryptionDialog);

        await report(status);

        expect(heading()).toBe(title);
        if (status.type === 'RecoveryKeyPending') expect(screen.getByText(KEY)).toBeInTheDocument();
    });

    it('keeps a half-typed key through a reconnect', async () => {
        const user = userEvent.setup();
        render(EncryptionDialog);
        await report({ type: 'NeedsRecoveryKey' });
        await user.type(screen.getByRole('textbox', { name: 'Recovery key' }), 'EsTc ab');

        resetMatrixSession();
        await tick();
        expect(heading()).toBeNull();
        matrixStatus.set('connected');
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

        await user.click(screen.getByRole('button', { name: 'Not Now' }));
        expect(heading()).toBeNull();
    });

    it('offers no way past a new key except confirming it was saved', async () => {
        const user = userEvent.setup();
        render(EncryptionDialog);
        await report({ type: 'RecoveryKeyPending', data: { key: KEY } });

        await user.keyboard('{Escape}');

        expect(heading()).toBe('Save Your Recovery Key');
        expect(screen.queryByRole('button', { name: 'Not Now' })).not.toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Close dialog' })).not.toBeInTheDocument();

        await user.click(screen.getByRole('button', { name: 'I have saved it' }));
        expect(sent('ConfirmRecoveryKeySaved')).toHaveLength(1);
    });

    it('resets only on its own button, once a password is typed', async () => {
        const user = userEvent.setup();
        render(EncryptionDialog);
        await report({ type: 'NeedsRecoveryKey' });

        await user.click(screen.getByRole('button', { name: "I don't have my key" }));
        await user.click(screen.getByRole('button', { name: 'Reset Encryption' }));

        expect(heading()).toBe('Reset Encryption');
        const reset = screen.getByRole('button', { name: 'Reset Encryption' });
        expect(reset).toBeDisabled();
        expect(screen.getByLabelText('Account password'), 'a new screen takes the focus with it').toHaveFocus();

        await user.type(screen.getByLabelText('Account password'), 'password{Enter}');
        expect(sent('ResetEncryption'), 'Enter must not start something that cannot be undone').toEqual([]);

        await user.click(reset);
        expect(sent('ResetEncryption')).toEqual([
            { type: 'Matrix', data: { type: 'ResetEncryption', data: { password: 'password' } } },
        ]);
    });

    it('replaces the key only once that is confirmed in the dialog', async () => {
        const user = userEvent.setup();
        render(SettingsAccount);
        render(EncryptionDialog);
        await report({ type: 'Ready' });
        const row = within(screen.getByRole('group', { name: 'Recovery Key' }));
        const dialog = () => within(screen.getByRole('dialog', { name: 'Replace Your Recovery Key' }));

        await user.click(row.getByRole('button', { name: 'Replace Recovery Key' }));
        await user.click(dialog().getByRole('button', { name: 'Cancel' }));
        expect(screen.queryByRole('dialog')).toBeNull();
        expect(sent('CreateRecoveryKey'), 'the current key must not stop working on one click').toEqual([]);

        await user.click(row.getByRole('button', { name: 'Replace Recovery Key' }));
        await user.click(dialog().getByRole('button', { name: 'Replace Recovery Key' }));
        expect(sent('CreateRecoveryKey')).toHaveLength(1);

        await report({ type: 'RecoveryKeyPending', data: { key: KEY } });
        expect(screen.getByRole('dialog', { name: 'Save Your Recovery Key' })).toHaveTextContent(KEY);
    });
});
