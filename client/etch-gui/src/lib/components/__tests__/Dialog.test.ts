import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { tick } from 'svelte';
import { resetStores } from '$lib/stores/__tests__/helpers';
import { connectingBookmark, passwordRequested, openSignOut } from '$lib/stores/servers';
import { certChangeRequest } from '$lib/stores/voiceState';
import CertDialog from '../CertDialog.svelte';
import PasswordDialog from '../PasswordDialog.svelte';
import SignOutDialog from '../SignOutDialog.svelte';

function backdropOf(name: string): HTMLElement {
    return screen.getByRole('dialog', { name }).parentElement!;
}

beforeEach(() => {
    resetStores();
});

describe('dialogs built on the shared shell', () => {
    it('puts the one opened last on top, whatever their order on the page', async () => {
        const user = userEvent.setup();
        // The password prompt comes before the sign out dialog on the page, as in the app.
        render(PasswordDialog);
        render(SignOutDialog);
        connectingBookmark.set({
            id: 'bk1', label: 'My Server', address: 'example.org', port: 443,
            username: 'someone', auto_connect: false,
            mumble_host: null, mumble_port: null, mumble_username: null, mumble_password: null,
        });
        openSignOut();
        await tick();
        passwordRequested.set(true);
        await tick();

        expect(Number(backdropOf('Password Required').style.zIndex))
            .toBeGreaterThan(Number(backdropOf('Sign Out').style.zIndex));
        expect(screen.getByLabelText('Password')).toHaveFocus();

        await user.keyboard('{Escape}');
        expect(screen.queryByRole('dialog', { name: 'Password Required' }), 'Escape belongs to the dialog on top').toBeNull();
        expect(screen.getByRole('dialog', { name: 'Sign Out' }), 'and to that one alone').toBeInTheDocument();
    });

    it('does not let Enter accept a changed certificate', async () => {
        render(CertDialog);
        certChangeRequest.set({ host: 'voice.example.org', port: 64738, new_fingerprint: 'aabbcc' });
        await tick();

        expect(screen.getByRole('button', { name: 'Reject' })).toHaveFocus();
    });
});
