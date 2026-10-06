<script lang="ts">
    import {
        signOutDialogOpen, signingOut, signOutError, signOut, closeSignOut,
        connectingBookmark, encryptionStatus,
    } from '$lib/stores';
    import Dialog from './Dialog.svelte';
    import DialogStatus from './DialogStatus.svelte';

    let recoveryKeyNotSaved = false;

    $: status = $encryptionStatus.type;
    // A failed sign out reconnects through Unknown, which must not change what the open dialog says.
    $: if (status !== 'Unknown' || !$signOutDialogOpen) {
        // Until a key is saved, this device holds the only copy of what unlocks the encrypted messages.
        recoveryKeyNotSaved = status === 'NeedsRecoverySetup' || status === 'RecoveryKeyPending';
    }
</script>

{#if $signOutDialogOpen}
    <Dialog title="Sign Out" dismissable={!$signingOut} on:dismiss={closeSignOut}>
        {#if $connectingBookmark}
            <p class="prompt">
                This signs <strong>{$connectingBookmark.username}</strong> out of <strong>{$connectingBookmark.address}</strong> on this device and ends your voice connection.
            </p>
        {:else}
            <p class="prompt">This signs you out on this device and ends your voice connection.</p>
        {/if}
        {#if recoveryKeyNotSaved}
            <p class="prompt danger" role="alert">
                You have not saved a recovery key. If you sign out now, you will not be able to read your encrypted messages after you sign back in.
            </p>
        {:else}
            <p class="prompt">
                You will need your password to sign back in, and your recovery key to read your encrypted messages again.
            </p>
        {/if}
        <DialogStatus error={$signOutError} note={$signingOut ? 'Signing out...' : ''} />
        <div class="actions">
            <button class="action-btn" class:danger={recoveryKeyNotSaved} on:click={signOut} disabled={$signingOut}>
                {$signOutError ? 'Try Again' : recoveryKeyNotSaved ? 'Sign Out Anyway' : 'Sign Out'}
            </button>
            <button
                class="action-btn secondary"
                on:click={closeSignOut}
                disabled={$signingOut}
                data-initial-focus={recoveryKeyNotSaved ? '' : undefined}
            >Cancel</button>
        </div>
    </Dialog>
{/if}
