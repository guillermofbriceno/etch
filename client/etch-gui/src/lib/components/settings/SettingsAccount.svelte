<script lang="ts">
    import {
        currentUser, canSetNameColor, matrixConnected,
        encryptionStatus, encryptionError, encryptionBusy,
        createRecoveryKey, showEncryptionPrompt, openEncryptionReset,
        signingOut, signOutError, signOut,
    } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';
    import type { EncryptionStatus } from '$lib/types';
    import Icon from '../Icon.svelte';
    import AvatarFallback from '../AvatarFallback.svelte';
    import NameColorSetting from './NameColorSetting.svelte';
    import { resolveMediaUrl, getInitial } from '$lib/media';
    import { open } from '@tauri-apps/plugin-dialog';

    let displayNameInput = $currentUser.displayName ?? $currentUser.username;
    let displayNameLabel = 'Apply';
    $: displayNameChanged = displayNameInput.trim() !== '' && displayNameInput.trim() !== ($currentUser.displayName ?? $currentUser.username);

    let currentPassword = '';
    let newPassword = '';
    let confirmPassword = '';
    let passwordLabel = 'Change Password';
    let passwordError = '';
    $: passwordValid = currentPassword.length > 0 && newPassword.length > 0 && newPassword === confirmPassword;

    const ENCRYPTION_SUMMARY: Record<EncryptionStatus['type'], string> = {
        Unknown: "Etch is still checking this account's encryption.",
        Ready: 'This device can read your encrypted messages. If you generate a new recovery key, the old one stops working.',
        NeedsRecoverySetup: 'You have not saved a recovery key yet.',
        RecoveryKeyPending: 'Your new recovery key is waiting to be saved.',
        NeedsRecoveryKey: 'This device needs your recovery key to read your encrypted messages.',
        NeedsVerifiedDevice: 'This device cannot read your encrypted messages yet, and your account has no recovery key. Create one on another device where you are signed in, or reset encryption.',
    };
    $: encryptionState = $encryptionStatus.type;
    $: encryptionSummary = $matrixConnected ? ENCRYPTION_SUMMARY[encryptionState] : 'Connect to a server to manage encryption.';
    $: canResetEncryption = $matrixConnected && encryptionState !== 'Unknown' && encryptionState !== 'RecoveryKeyPending';

    let confirmingSignOut = false;
    // Until a key is saved, this device holds the only copy of what unlocks the encrypted messages.
    $: recoveryKeyNotSaved = encryptionState === 'NeedsRecoverySetup' || encryptionState === 'RecoveryKeyPending';
    $: if (!$matrixConnected) confirmingSignOut = false;

    function confirmSignOut() {
        confirmingSignOut = false;
        signOut();
    }

    async function pickAvatar() {
        const path = await open({
            filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp'] }],
            multiple: false,
        });
        if (path) {
            sendCoreCommand({ type: 'Matrix', data: { type: 'SetAvatar', data: path } });
        }
    }

    function changePassword() {
        if (newPassword !== confirmPassword) {
            passwordError = 'Passwords do not match';
            return;
        }
        passwordError = '';
        sendCoreCommand({ type: 'Matrix', data: { type: 'ChangePassword', data: { current_password: currentPassword, new_password: newPassword } } });
        currentPassword = '';
        newPassword = '';
        confirmPassword = '';
        passwordLabel = 'Saved!';
        setTimeout(() => passwordLabel = 'Change Password', 2000);
    }

    function applyDisplayName() {
        const name = displayNameInput.trim();
        if (!name) return;
        sendCoreCommand({ type: 'Matrix', data: { type: 'SetDisplayName', data: name } });
        currentUser.update(u => ({ ...u, displayName: name }));
        displayNameLabel = 'Saved!';
        setTimeout(() => displayNameLabel = 'Apply', 2000);
    }
</script>

<div class="tab-pane">
    <h2>My Account</h2>

    <div class="profile-row">
        <button class="avatar-edit-wrapper" on:click={pickAvatar}>
            {#if $currentUser.avatarUrl}
                <img src={resolveMediaUrl($currentUser.avatarUrl)} alt="avatar" class="profile-avatar" />
            {:else}
                <AvatarFallback initial={getInitial($currentUser.displayName ?? $currentUser.username)} size={72} fontSize={28} />
            {/if}
            <div class="avatar-edit-overlay">
                <Icon name="edit" size={16} />
            </div>
        </button>

        <div class="profile-fields">
            <div class="setting-group">
                <label for="display-name">Display Name</label>
                <div class="action-row">
                    <input type="text" id="display-name" class="text-input display-name-input" bind:value={displayNameInput} placeholder="Enter display name" />
                    <button class="action-btn" on:click={applyDisplayName} disabled={!displayNameChanged}>{displayNameLabel}</button>
                </div>
            </div>

            {#if $canSetNameColor}
                <NameColorSetting />
            {/if}
        </div>
    </div>

    <div class="divider"></div>

    <div class="setting-group">
        <label for="current-password">Change Password</label>
        <input type="password" id="current-password" class="text-input password-input" bind:value={currentPassword} placeholder="Current password" />
        <input type="password" class="text-input password-input" bind:value={newPassword} placeholder="New password" />
        <input type="password" class="text-input password-input" bind:value={confirmPassword} placeholder="Confirm new password" />
        {#if passwordError}
            <span class="password-error">{passwordError}</span>
        {/if}
        <button class="action-btn" on:click={changePassword} disabled={!passwordValid}>{passwordLabel}</button>
    </div>

    <div class="divider"></div>

    <div class="setting-group" role="group" aria-label="Encryption">
        <span class="setting-label">Encryption</span>
        <p class="setting-desc">{encryptionSummary}</p>
        {#if $encryptionError}
            <span class="password-error" role="alert">{$encryptionError}</span>
        {/if}
        <div class="action-row">
            {#if $matrixConnected && encryptionState === 'Ready'}
                <button class="action-btn secondary" on:click={createRecoveryKey} disabled={$encryptionBusy}>Generate a new recovery key</button>
            {:else if $matrixConnected && encryptionState === 'NeedsRecoverySetup'}
                <button class="action-btn" on:click={showEncryptionPrompt}>Create recovery key</button>
            {:else if $matrixConnected && encryptionState === 'NeedsRecoveryKey'}
                <button class="action-btn" on:click={showEncryptionPrompt}>Enter recovery key</button>
            {/if}
            <button class="action-btn danger" on:click={openEncryptionReset} disabled={!canResetEncryption}>Reset encryption</button>
        </div>
    </div>

    <div class="divider"></div>

    <div class="setting-group" role="group" aria-label="Sign out">
        <span class="setting-label">Sign out</span>
        {#if confirmingSignOut}
            {#if recoveryKeyNotSaved}
                <p class="setting-desc sign-out-warning" role="alert">
                    You have not saved a recovery key. If you sign out now, you will not be able to read your encrypted messages after you sign back in.
                </p>
            {:else}
                <p class="setting-desc">
                    You will need your password to sign back in, and your recovery key to read your encrypted messages again.
                </p>
            {/if}
            <div class="action-row">
                <button class="action-btn" class:danger={recoveryKeyNotSaved} on:click={confirmSignOut}>
                    {recoveryKeyNotSaved ? 'Sign out anyway' : 'Sign out'}
                </button>
                <button class="action-btn secondary" on:click={() => confirmingSignOut = false}>Cancel</button>
            </div>
        {:else}
            <p class="setting-desc">Signing out removes this device from your account and ends your voice connection.</p>
            {#if $signOutError}
                <span class="password-error" role="alert">{$signOutError}</span>
            {/if}
            <div class="action-row">
                <button class="action-btn secondary" on:click={() => confirmingSignOut = true} disabled={!$matrixConnected || $signingOut}>
                    {$signingOut ? 'Signing out...' : 'Sign out'}
                </button>
            </div>
        {/if}
    </div>
</div>

<style>
    .profile-row { display: flex; align-items: flex-start; gap: 20px; }
    .profile-fields { flex: 1; min-width: 0; }

    .avatar-edit-wrapper {
        position: relative;
        width: 72px;
        height: 72px;
        border-radius: 50%;
        flex-shrink: 0;
        cursor: pointer;
        border: none;
        padding: 0;
        background: none;
        margin-top: 20px;
    }

    .profile-avatar {
        width: 72px;
        height: 72px;
        border-radius: 50%;
        object-fit: cover;
    }


    .avatar-edit-overlay {
        position: absolute;
        inset: 0;
        border-radius: 50%;
        background-color: rgba(0, 0, 0, 0.6);
        display: flex;
        align-items: center;
        justify-content: center;
        color: var(--text-bright);
        opacity: 0;
        transition: opacity 0.15s;
    }

    .avatar-edit-wrapper:hover .avatar-edit-overlay { opacity: 1; }

    .display-name-input { flex: 1; }
    .password-input { margin-bottom: 8px; }
    .password-error { color: var(--status-danger); font-size: 13px; margin-bottom: 4px; }
    p.setting-desc.sign-out-warning { color: var(--status-danger); }
</style>
