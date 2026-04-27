<script lang="ts">
    import { closeOverlay, settingsTab, currentUser, errorLog, sfxVolume, transmissionMode, setTransmissionMode, vadThreshold, setVadThreshold, voiceHold, setVoiceHold, useMumbleSettings, setUseMumbleSettings, activeChannel, showRoomIds, theme, updateStatus, updateVersion, updateError, checkForUpdate, restartApp } from '$lib/stores';
    import type { TransmissionMode, Theme, UpdateStatus } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';
    import { open } from '@tauri-apps/plugin-dialog';
    import { getVersion } from '@tauri-apps/api/app';

    let activeTab = $settingsTab;
    let appVersion = '';
    let extraMumbleArgs = '';
    let devMode = false;
    let dmTargetUserId = '';
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

    getVersion().then(v => appVersion = v);

    function onInputProfileChange(mode: TransmissionMode) {
        setTransmissionMode(mode);
    }

    function openMumbleGui() {
        sendCoreCommand({ type: 'System', data: { type: 'OpenMumbleGui', data: extraMumbleArgs } });
    }

    function restartMumble() {
        sendCoreCommand({ type: 'System', data: { type: 'RestartMumble', data: extraMumbleArgs } });
    }

    let copyLabel = 'Copy All';

    async function copyErrorLog() {
        const text = $errorLog.map(e =>
            `[${e.timestamp.toLocaleTimeString()}] [${e.target}] ${e.message}`
        ).join('\n');
        await navigator.clipboard.writeText(text);
        copyLabel = 'Copied!';
        setTimeout(() => copyLabel = 'Copy All', 2000);
    }

    function handleKeydown(event: KeyboardEvent) {
        if (event.key === 'Escape') closeOverlay();
    }
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="settings-layout">
    <div class="settings-sidebar">
        <div class="sidebar-content">
            <h3 class="group-header">User Settings</h3>
            <button class="tab {activeTab === 'account'    ? 'active' : ''}" on:click={() => activeTab = 'account'}>My Account</button>
            <button class="tab {activeTab === 'appearance' ? 'active' : ''}" on:click={() => activeTab = 'appearance'}>Appearance</button>

            <div class="divider"></div>

            <h3 class="group-header">App Settings</h3>
            <button class="tab {activeTab === 'voice'    ? 'active' : ''}" on:click={() => activeTab = 'voice'}>Voice & Audio</button>
            <button class="tab {activeTab === 'keybinds' ? 'active' : ''}" on:click={() => activeTab = 'keybinds'}>Keybinds</button>
            <button class="tab {activeTab === 'updates'  ? 'active' : ''}" on:click={() => activeTab = 'updates'}>Updates</button>
            <button class="tab {activeTab === 'advanced' ? 'active' : ''}" on:click={() => activeTab = 'advanced'}>Advanced</button>
            {#if devMode}
                <button class="tab {activeTab === 'developer' ? 'active' : ''}" on:click={() => activeTab = 'developer'}>Developer</button>
            {/if}
            <button class="tab {activeTab === 'errors' ? 'active' : ''}" on:click={() => activeTab = 'errors'}>
                Error Log
                {#if $errorLog.length > 0}
                    <span class="error-badge">{$errorLog.length}</span>
                {/if}
            </button>

            <div class="version-info">Etch v{appVersion}</div>
        </div>
    </div>

    <div class="settings-content">
        <div class="content-container">

            {#if activeTab === 'voice'}
                <div class="tab-pane">
                    <h2>Voice & Audio</h2>

                    <h3 class="section-header">Voice</h3>

                    <div class="setting-group">
                        <label class="checkbox-option">
                            <input type="checkbox" checked={$useMumbleSettings} on:change={(e) => setUseMumbleSettings(e.currentTarget.checked)} />
                            <span class="checkbox-label">Use Mumble's Settings</span>
                        </label>
                        <p class="setting-desc">Defer to Mumble's built-in voice settings. Requires Restart.</p>
                    </div>

                    <div class="setting-group" class:disabled={$useMumbleSettings}>
                        <label>Input Profile</label>
                        <div class="radio-group">
                            <label class="radio-option">
                                <input type="radio" name="input-profile" value="voice_activation" checked={$transmissionMode === 'voice_activation'} on:change={() => onInputProfileChange('voice_activation')} />
                                <div class="radio-content">
                                    <span class="radio-label">Voice Isolation</span>
                                    <span class="radio-desc">Automatically transmits when you speak</span>
                                </div>
                            </label>
                            <label class="radio-option">
                                <input type="radio" name="input-profile" value="continuous" checked={$transmissionMode === 'continuous'} on:change={() => onInputProfileChange('continuous')} />
                                <div class="radio-content">
                                    <span class="radio-label">Continuous</span>
                                    <span class="radio-desc">Always transmitting audio</span>
                                </div>
                            </label>
                            <label class="radio-option">
                                <input type="radio" name="input-profile" value="push_to_talk" checked={$transmissionMode === 'push_to_talk'} on:change={() => onInputProfileChange('push_to_talk')} />
                                <div class="radio-content">
                                    <span class="radio-label">Push to Talk</span>
                                    <span class="radio-desc">Transmits only while a key is held</span>
                                </div>
                            </label>
                        </div>
                    </div>

                    <div class="setting-group volume-slider" class:disabled={$useMumbleSettings || $transmissionMode !== 'voice_activation'}>
                        <label for="speech-threshold">Speech Threshold</label>
                        <div class="slider-container">
                            <input type="range" id="speech-threshold" min="0" max="100" value={$vadThreshold} on:change={(e) => setVadThreshold(+e.currentTarget.value)} class="range-input" disabled={$useMumbleSettings || $transmissionMode !== 'voice_activation'} />
                            <span class="volume-readout">{$vadThreshold}%</span>
                        </div>
                    </div>

                    <div class="setting-group volume-slider" class:disabled={$useMumbleSettings}>
                        <label for="voice-hold">Voice Hold</label>
                        <div class="slider-container">
                            <input type="range" id="voice-hold" min="50" max="1000" step="10" value={$voiceHold} on:change={(e) => setVoiceHold(+e.currentTarget.value)} class="range-input" />
                            <span class="volume-readout">{$voiceHold}ms</span>
                        </div>
                    </div>

                    <h3 class="section-header">Audio</h3>

                    <div class="setting-group volume-slider">
                        <label for="sfx-volume">Sound Effects Volume</label>
                        <div class="slider-container">
                            <input type="range" id="sfx-volume" min="0" max="100" bind:value={$sfxVolume} class="range-input" />
                            <span class="volume-readout">{$sfxVolume}%</span>
                        </div>
                    </div>
                </div>

            {:else if activeTab === 'advanced'}
                <div class="tab-pane">
                    <h2>Advanced</h2>

                    <div class="setting-group">
                        <label for="extra-args">Extra Mumble Launch Options</label>
                        <input type="text" id="extra-args" class="text-input" bind:value={extraMumbleArgs} placeholder="e.g. --print-supported-audio-formats" />
                    </div>

                    <div class="setting-group">
                        <label>Mumble Client</label>
                        <p class="setting-desc">Restart the voice backend or open the Mumble GUI for direct access to audio settings, certificates, and diagnostics. Both will restart the voice connection.</p>
                        <div class="action-row">
                            <button class="action-btn" on:click={restartMumble}>Restart Mumble</button>
                            <button class="action-btn secondary" on:click={openMumbleGui}>Open Mumble GUI</button>
                        </div>
                    </div>

                    <div class="setting-group">
                        <label class="checkbox-option">
                            <input type="checkbox" bind:checked={devMode} />
                            <span class="checkbox-label">Enable Developer Options</span>
                        </label>
                    </div>
                </div>

            {:else if activeTab === 'developer'}
                <div class="tab-pane">
                    <h2>Developer Options</h2>

                    <div class="setting-group">
                        <label>Error Reporting</label>
                        <p class="setting-desc">Send a mock error through the logging pipeline to verify it appears in the Error Log.</p>
                        <button class="action-btn" on:click={() => sendCoreCommand({ type: 'System', data: { type: 'TestError' } })}>Send Test Error</button>
                    </div>

                    <div class="setting-group">
                        <label>Log Level</label>
                        <p class="setting-desc">Change the runtime log verbosity. Set ETCH_LOG env var to change the startup default.</p>
                        <select class="hardware-select" on:change={(e) => sendCoreCommand({ type: 'System', data: { type: 'SetLogLevel', data: e.currentTarget.value } })}>
                            <option value="error">Error</option>
                            <option value="warn">Warn</option>
                            <option value="info">Info</option>
                            <option value="debug" selected>Debug</option>
                            <option value="trace">Trace</option>
                        </select>
                    </div>

                    <div class="setting-group">
                        <label>Create DM</label>
                        <p class="setting-desc">Start a direct message with a user by their full Matrix ID.</p>
                        <div class="dm-row">
                            <input type="text" bind:value={dmTargetUserId} placeholder="@alice:etchtest.emptytincan.com" class="dm-input" />
                            <button class="action-btn" on:click={() => {
                                if (dmTargetUserId.trim()) {
                                    sendCoreCommand({ type: 'Matrix', data: { type: 'CreateDirectMessage', data: { target_user_id: dmTargetUserId.trim() } } });
                                }
                            }}>Send DM</button>
                        </div>
                    </div>

                    <div class="setting-group">
                        <label>Display</label>
                        <label class="checkbox-row">
                            <input type="checkbox" bind:checked={$showRoomIds} />
                            <span class="checkbox-label">Show Room IDs in chat header</span>
                        </label>
                    </div>

                    <div class="setting-group">
                        <label>Encryption</label>
                        <p class="setting-desc">Enable encryption on the currently active room.</p>
                        {#if !$activeChannel?.is_encrypted}
                            <p class="setting-desc" style="color: #ed4245;">This is irreversible, do you have permission to do this?</p>
                        {/if}
                        <button class="action-btn danger" disabled={!$activeChannel || $activeChannel.is_encrypted} on:click={() => {
                            if ($activeChannel) {
                                sendCoreCommand({ type: 'Matrix', data: { type: 'EnableEncryption', data: { room_id: $activeChannel.id } } });
                            }
                        }}>{$activeChannel?.is_encrypted ? 'Already encrypted' : $activeChannel ? `Encrypt "${$activeChannel.display_name}"` : 'No room selected'}</button>
                    </div>
                </div>

            {:else if activeTab === 'errors'}
                <div class="tab-pane">
                    <div class="tab-header">
                        <h2>Error Log</h2>
                        {#if $errorLog.length > 0}
                            <button class="action-btn secondary" on:click={copyErrorLog}>{copyLabel}</button>
                        {/if}
                    </div>
                    {#if $errorLog.length === 0}
                        <p class="placeholder-text">No errors recorded.</p>
                    {:else}
                        <div class="error-log-list">
                            {#each $errorLog as entry}
                                <div class="error-entry">
                                    <div class="error-meta">
                                        <span class="error-timestamp">{entry.timestamp.toLocaleTimeString()}</span>
                                        <span class="error-target">{entry.target}</span>
                                    </div>
                                    <div class="error-message">{entry.message}</div>
                                </div>
                            {/each}
                        </div>
                    {/if}
                </div>

            {:else if activeTab === 'account'}
                <div class="tab-pane">
                    <h2>My Account</h2>

                    <div class="profile-row">
                        <button class="avatar-edit-wrapper" on:click={pickAvatar}>
                            {#if $currentUser.avatarUrl}
                                <img src={$currentUser.avatarUrl.replace('mxc://', 'etch-media://')} alt="avatar" class="profile-avatar" />
                            {:else}
                                <div class="profile-avatar profile-avatar-fallback">{($currentUser.displayName ?? $currentUser.username ?? '?').charAt(0).toUpperCase()}</div>
                            {/if}
                            <div class="avatar-edit-overlay">
                                <svg width="16" height="16" viewBox="0 0 24 24">
                                    <path fill="currentColor" d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1.003 1.003 0 0 0 0-1.42l-2.34-2.33a1.003 1.003 0 0 0-1.42 0l-1.83 1.83 3.75 3.75 1.84-1.83z" />
                                </svg>
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
            {:else if activeTab === 'appearance'}
                <div class="tab-pane">
                    <h2>Appearance</h2>

                    <div class="setting-group">
                        <label>Theme</label>
                        <select class="hardware-select" value={$theme} on:change={(e) => theme.set(e.currentTarget.value as Theme)}>
                            <option value="default">Default</option>
                            <option value="oled">OLED Optimized</option>
                        </select>
                    </div>
                </div>
            {:else if activeTab === 'updates'}
                <div class="tab-pane">
                    <h2>Updates</h2>

                    <div class="setting-group">
                        <p class="setting-desc">Current version: {appVersion}</p>

                        {#if $updateStatus === 'idle'}
                            <button class="action-btn" on:click={checkForUpdate}>Check for Updates</button>
                        {:else if $updateStatus === 'checking'}
                            <button class="action-btn" disabled>Checking...</button>
                        {:else if $updateStatus === 'available'}
                            <p class="update-info">Downloading v{$updateVersion}...</p>
                        {:else if $updateStatus === 'ready'}
                            <p class="update-info">v{$updateVersion} is ready. Restart to apply.</p>
                            <button class="action-btn" on:click={restartApp}>Restart Now</button>
                        {:else if $updateStatus === 'up_to_date'}
                            <p class="update-info">You're on the latest version.</p>
                            <button class="action-btn secondary" on:click={checkForUpdate}>Check Again</button>
                        {:else if $updateStatus === 'error'}
                            <p class="update-error">{$updateError}</p>
                            <button class="action-btn" on:click={checkForUpdate}>Retry</button>
                        {/if}
                    </div>
                </div>
            {:else}
                <div class="tab-pane">
                    <h2>{activeTab.charAt(0).toUpperCase() + activeTab.slice(1)}</h2>
                    <p class="placeholder-text">Not implemented yet.</p>
                </div>
            {/if}

        </div>

        <div class="close-action">
            <button class="close-btn" on:click={closeOverlay} aria-label="Close">
                <svg width="18" height="18" viewBox="0 0 24 24">
                    <path fill="currentColor" d="M18.4 4L12 10.4L5.6 4L4 5.6L10.4 12L4 18.4L5.6 20L12 13.6L18.4 20L20 18.4L13.6 12L20 5.6L18.4 4Z" />
                </svg>
            </button>
            <span class="esc-hint">ESC</span>
        </div>
    </div>
</div>

<style>
    .settings-layout {
        display: flex;
        width: 100%;
        height: 100%;
        background-color: var(--bg-primary);
    }

    .settings-sidebar {
        flex: 1 1 auto;
        display: flex;
        justify-content: flex-end;
        background-color: var(--bg-secondary);
        padding-top: 60px;
        padding-right: 20px;
    }

    .sidebar-content { width: 218px; display: flex; flex-direction: column; flex: 1; }

    .version-info {
        margin-top: auto;
        padding: 10px 10px 20px;
        font-size: 12px;
        color: #4f5660;
    }

    .group-header {
        font-size: 12px;
        font-weight: 700;
        color: #8e9297;
        text-transform: uppercase;
        margin: 0 0 8px 10px;
        letter-spacing: 0.2px;
    }

    .tab {
        background: transparent;
        border: none;
        color: #b9bbbe;
        text-align: left;
        padding: 6px 10px;
        margin-bottom: 2px;
        border-radius: 4px;
        font-size: 16px;
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: background-color 0.15s, color 0.15s;
    }

    .tab:hover { background-color: var(--bg-hover); color: #dcddde; }
    .tab.active { background-color: var(--bg-active); color: #fff; }

    .divider { height: 1px; background-color: var(--bg-active); margin: 10px 10px 14px 10px; }

    .settings-content {
        flex: 1 1 800px;
        display: flex;
        position: relative;
        background-color: var(--bg-primary);
        padding-top: 60px;
        padding-left: 40px;
        overflow-y: auto;
    }

    .settings-content::-webkit-scrollbar { width: 6px; }
    .settings-content::-webkit-scrollbar-track { background: transparent; }
    .settings-content::-webkit-scrollbar-thumb { background-color: #202225; border-radius: 3px; }

    .content-container { width: 100%; max-width: 740px; padding-bottom: 60px; }

    .tab-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 20px; }
    .tab-header h2 { margin: 0; }
    .tab-pane h2 { color: #fff; font-size: 20px; font-weight: 600; margin-top: 0; margin-bottom: 20px; }

    .section-header {
        color: #fff;
        font-size: 13px;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.3px;
        margin: 8px 0 16px;
    }

    .section-header:not(:first-of-type) { margin-top: 20px; }

    .placeholder-text { color: #b9bbbe; }

    .setting-group { margin-bottom: 24px; display: flex; flex-direction: column; }

    .setting-group label {
        color: #8e9297;
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        margin-bottom: 8px;
    }

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

    .device-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 20px; }

    .hardware-select {
        background-color: var(--bg-input);
        color: #dcddde;
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 10px;
        font-size: 16px;
        font-family: 'Inter', sans-serif;
        outline: none;
        appearance: none;
        cursor: pointer;
    }

    .hardware-select:focus { border-color: #7289da; }

    .configure-hotkey-btn {
        background-color: #5865f2;
        color: #fff;
        border: none;
        border-radius: 4px;
        padding: 10px 16px;
        font-size: 14px;
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: background-color 0.15s;
        align-self: flex-start;
    }

    .configure-hotkey-btn:hover { background-color: #4752c4; }

    .toggle-row { flex-direction: column; }

    .toggle-container { display: flex; align-items: center; gap: 12px; }

    .toggle-switch {
        position: relative;
        width: 44px;
        height: 24px;
        border-radius: 12px;
        border: none;
        background-color: rgba(255, 255, 255, 0.12);
        cursor: pointer;
        padding: 0;
        transition: background-color 0.2s;
        flex-shrink: 0;
    }

    .toggle-switch.active { background-color: #3ba55d; }

    .toggle-knob {
        position: absolute;
        top: 3px;
        left: 3px;
        width: 18px;
        height: 18px;
        border-radius: 50%;
        background-color: #fff;
        transition: transform 0.2s;
        pointer-events: none;
    }

    .toggle-switch.active .toggle-knob { transform: translateX(20px); }

    .toggle-label { color: #b9bbbe; font-size: 14px; }

    .slider-container { display: flex; align-items: center; gap: 16px; }

    .range-input { flex-grow: 1; cursor: pointer; }

    .volume-readout { color: #dcddde; font-size: 14px; min-width: 40px; }

    .setting-group.disabled { opacity: 0.4; pointer-events: none; }

    .setting-desc { color: #8e9297; font-size: 13px; margin: 0 0 12px; }

    .action-btn {
        background-color: #5865f2;
        color: #fff;
        border: none;
        border-radius: 4px;
        padding: 8px 16px;
        font-size: 14px;
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: background-color 0.15s;
        align-self: flex-start;
    }

    .action-btn:hover:not(:disabled) { background-color: #4752c4; }
    .action-btn:disabled { opacity: 0.4; cursor: default; }
    .action-btn.secondary { background-color: var(--bg-active); }
    .action-btn.secondary:hover { background-color: rgba(255, 255, 255, 0.12); }
    .action-btn.danger { background-color: #ed4245; }
    .action-btn.danger:hover:not(:disabled) { background-color: #c93b3e; }

    .action-row { display: flex; gap: 8px; }

    .text-input {
        background-color: var(--bg-input);
        color: #dcddde;
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 8px 10px;
        font-size: 14px;
        font-family: 'Inter', sans-serif;
        outline: none;
    }

    .display-name-input { flex: 1; }
    .password-input { margin-bottom: 8px; }
    .password-error { color: #ed4245; font-size: 13px; margin-bottom: 4px; }
    .text-input:focus { border-color: #5865f2; }
    .text-input::placeholder { color: #4f5660; }

    .dm-row { display: flex; gap: 8px; }
    .dm-input {
        flex: 1;
        background-color: var(--bg-input);
        color: #dcddde;
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 8px 10px;
        font-size: 14px;
        font-family: 'Inter', sans-serif;
        outline: none;
    }
    .dm-input:focus { border-color: #5865f2; }
    .dm-input::placeholder { color: #4f5660; }

    .close-action {
        flex-shrink: 0;
        margin-left: 20px;
        margin-right: 20px;
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 8px;
    }

    .close-btn {
        width: 36px;
        height: 36px;
        border-radius: 50%;
        background-color: transparent;
        border: 2px solid #72767d;
        color: #72767d;
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        transition: background-color 0.15s, color 0.15s, border-color 0.15s;
    }

    .close-btn:hover { background-color: rgba(255, 255, 255, 0.1); color: #dcddde; border-color: #dcddde; }

    .esc-hint { color: #72767d; font-size: 13px; font-weight: 600; }

    .error-badge {
        background-color: #ed4245;
        color: #fff;
        font-size: 11px;
        font-weight: 700;
        padding: 1px 5px;
        border-radius: 8px;
        margin-left: 6px;
    }

    .error-log-list {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }

    .error-entry {
        background-color: rgba(237, 66, 69, 0.08);
        border-left: 3px solid #ed4245;
        border-radius: 4px;
        padding: 10px 12px;
    }

    .error-meta {
        display: flex;
        gap: 10px;
        margin-bottom: 4px;
        font-size: 12px;
    }

    .error-timestamp { color: #72767d; }
    .error-target { color: #8e9297; font-family: monospace; }
    .error-message { color: #dcddde; font-size: 14px; word-break: break-word; -webkit-user-select: text; user-select: text; cursor: text; }
    .error-target { -webkit-user-select: text; user-select: text; cursor: text; }

    .radio-group { display: flex; flex-direction: column; gap: 2px; }

    .radio-option {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 6px 0;
        cursor: pointer;
    }

    .radio-option input[type="radio"] {
        accent-color: #5865f2;
        width: 16px;
        height: 16px;
        margin: 0;
        flex-shrink: 0;
    }

    .radio-content { display: flex; flex-direction: column; gap: 1px; }

    .radio-label { color: #dcddde; font-size: 14px; font-weight: 500; }

    .radio-label, .radio-desc { text-transform: none; }
    .radio-desc { color: #72767d; font-size: 12px; }

    .checkbox-option {
        display: flex;
        align-items: center;
        gap: 10px;
        cursor: pointer;
    }

    .checkbox-option input[type="checkbox"] {
        accent-color: #5865f2;
        width: 16px;
        height: 16px;
        margin: 0;
    }

    .checkbox-label { color: #dcddde; font-size: 14px; font-weight: 500; text-transform: none; }

    .update-info { color: #b9bbbe; font-size: 14px; margin: 0 0 12px; }
    .update-error { color: #ed4245; font-size: 14px; margin: 0 0 12px; }
</style>
