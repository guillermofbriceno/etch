import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { tick } from 'svelte';
import { invoke } from '@tauri-apps/api/core';
import { resetStores } from '$lib/stores/__tests__/helpers';
import { handleMatrixEvent as handleEncryptionEvent } from '$lib/stores/encryption';
import { connectingBookmark, handleSystemEvent } from '$lib/stores/servers';
import { matrixStatus } from '$lib/stores/matrixConnection';
import { resetMatrixSession, resetLoginSession } from '$lib/stores/session';
import type { CoreCommand } from '$lib/ipc';
import type { EncryptionStatus } from '$lib/types';
import SettingsModal from '../SettingsModal.svelte';
import SignOutDialog from '../SignOutDialog.svelte';

function signOutsSent(): CoreCommand[] {
    return vi.mocked(invoke).mock.calls
        .map(([, args]) => (args as { command: CoreCommand }).command)
        .filter((command) => command.type === 'System' && command.data.type === 'SignOut');
}

async function report(status: EncryptionStatus): Promise<void> {
    handleEncryptionEvent({ type: 'EncryptionStatus', data: status });
    await tick();
}

function navItem(): HTMLElement {
    return screen.getByRole('button', { name: 'Sign Out' });
}

function dialog() {
    return within(screen.getByRole('dialog', { name: 'Sign Out' }));
}

beforeEach(() => {
    resetStores();
    vi.mocked(invoke).mockClear();
    connectingBookmark.set({
        id: 'bk1', label: 'My Server', address: 'example.org', port: 443,
        username: 'someone', auto_connect: false,
        mumble_host: null, mumble_port: null, mumble_username: null, mumble_password: null,
    });
    render(SettingsModal);
    render(SignOutDialog);
});

describe('signing out from Settings', () => {
    it('asks before signing out, and warns only when no recovery key is saved', async () => {
        const user = userEvent.setup();
        expect(navItem(), 'there is no session to sign out of yet').toBeDisabled();
        expect(screen.getByText('Available once Etch is connected.')).toBeInTheDocument();

        matrixStatus.set('connected');
        await report({ type: 'Ready' });
        await user.click(navItem());
        expect(screen.getByRole('dialog')).toHaveTextContent('This signs someone out of example.org on this device');
        expect(dialog().getByText(/You will need your password to sign back in, and your recovery key/)).toBeInTheDocument();
        expect(dialog().queryByRole('alert'), 'a user whose key is saved must not be alarmed').toBeNull();
        expect(dialog().getByRole('button', { name: 'Sign Out' })).toHaveFocus();
        await user.tab();
        await user.tab();
        expect(dialog().getByRole('button', { name: 'Sign Out' }), 'Tab must not reach what is behind the dialog').toHaveFocus();
        expect(signOutsSent(), 'nothing is sent until it is confirmed').toEqual([]);

        await user.click(dialog().getByRole('button', { name: 'Cancel' }));
        expect(screen.queryByRole('dialog')).toBeNull();
        expect(navItem(), 'focus goes back to where the dialog was opened from').toHaveFocus();

        await report({ type: 'NeedsRecoverySetup' });
        await user.click(navItem());
        expect(dialog().getByRole('alert')).toHaveTextContent(
            'You have not saved a recovery key. If you sign out now, you will not be able to read your encrypted messages after you sign back in.',
        );
        expect(dialog().getByRole('button', { name: 'Cancel' }), 'Enter must not sign out a user who was just warned').toHaveFocus();
        await user.click(dialog().getByRole('button', { name: 'Sign Out Anyway' }));

        expect(signOutsSent()).toEqual([{ type: 'System', data: { type: 'SignOut' } }]);
        expect(dialog().getByRole('status')).toHaveTextContent('Signing out...');
        expect(dialog().getByRole('button', { name: 'Sign Out Anyway' })).toBeDisabled();
    });

    it('stays open with the reason when signing out fails', async () => {
        const user = userEvent.setup();
        matrixStatus.set('connected');
        await report({ type: 'Ready' });
        await user.click(navItem());
        await user.click(dialog().getByRole('button', { name: 'Sign Out' }));

        // Core reconnects first, so the session is reset before the failure is reported.
        resetMatrixSession();
        handleSystemEvent({ type: 'SignOutFailed', data: { reason: 'Could not reach the server.' } });
        resetMatrixSession();
        await tick();

        expect(dialog().getByRole('alert'), 'a later reconnect attempt must not take the reason away')
            .toHaveTextContent('Could not reach the server.');

        await user.click(dialog().getByRole('button', { name: 'Try Again' }));
        expect(signOutsSent()).toHaveLength(2);

        // The router ends the login on SignedOut.
        resetLoginSession();
        await tick();
        expect(screen.queryByRole('dialog')).toBeNull();
    });
});
