<script lang="ts">
    import { currentUser } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';
    import Icon from '../Icon.svelte';
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
                <img src={$currentUser.avatarUrl.startsWith('mxc://') ? $currentUser.avatarUrl.replace('mxc://', 'etch-media://') : $currentUser.avatarUrl} alt="avatar" class="profile-avatar" />
            {:else}
                <div class="profile-avatar profile-avatar-fallback">{($currentUser.displayName ?? $currentUser.username ?? '?').charAt(0).toUpperCase()}</div>
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

    .profile-avatar-fallback {
        display: flex;
        align-items: center;
        justify-content: center;
        background-color: #5865f2;
        color: #fff;
        font-weight: 600;
        font-size: 28px;
    }

    .avatar-edit-overlay {
        position: absolute;
        inset: 0;
        border-radius: 50%;
        background-color: rgba(0, 0, 0, 0.6);
        display: flex;
        align-items: center;
        justify-content: center;
        color: #fff;
        opacity: 0;
        transition: opacity 0.15s;
    }

    .avatar-edit-wrapper:hover .avatar-edit-overlay { opacity: 1; }

    .display-name-input { flex: 1; }
    .password-input { margin-bottom: 8px; }
    .password-error { color: #ed4245; font-size: 13px; margin-bottom: 4px; }
</style>
