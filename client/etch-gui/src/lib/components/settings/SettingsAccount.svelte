<script lang="ts">
    import { currentUser, canSetNameColor, matrixConnected, encryptionStatus, openEncryptionDialog } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';
    import { encryptionText, NOT_CONNECTED } from '$lib/encryptionText';
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

    $: recoveryKey = encryptionText($encryptionStatus.type);

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

    <h3 class="section-header">Profile</h3>

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

    <h3 class="section-header">Password and Security</h3>

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

    <div class="setting-group" role="group" aria-labelledby="recovery-key-label">
        <span class="setting-label" id="recovery-key-label">Recovery Key</span>
        {#if $matrixConnected}
            <p class="setting-desc">{recoveryKey.summary}</p>
            {#if recoveryKey.button}
                <button class="action-btn" class:secondary={$encryptionStatus.type === 'Ready'} on:click={openEncryptionDialog}>
                    {recoveryKey.button}
                </button>
            {/if}
        {:else}
            <p class="setting-desc">{NOT_CONNECTED}</p>
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
</style>
