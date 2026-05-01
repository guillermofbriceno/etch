<script lang="ts">
    import { closeOverlay, activeChannel, showRoomIds } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';
    import { injectDemoData } from '$lib/demoData';

    let dmTargetUserId = '';
</script>

<div class="tab-pane">
    <h2>Developer Options</h2>

    <div class="setting-group">
        <span class="setting-label">Error Reporting</span>
        <p class="setting-desc">Send a mock error through the logging pipeline to verify it appears in the Error Log.</p>
        <button class="action-btn" on:click={() => sendCoreCommand({ type: 'System', data: { type: 'TestError' } })}>Send Test Error</button>
    </div>

    <div class="setting-group">
        <label for="log-level-select">Log Level</label>
        <p class="setting-desc">Change the runtime log verbosity. Set ETCH_LOG env var to change the startup default.</p>
        <select id="log-level-select" class="hardware-select" on:change={(e) => sendCoreCommand({ type: 'System', data: { type: 'SetLogLevel', data: e.currentTarget.value } })}>
            <option value="error">Error</option>
            <option value="warn">Warn</option>
            <option value="info">Info</option>
            <option value="debug" selected>Debug</option>
            <option value="trace">Trace</option>
        </select>
    </div>

    <div class="setting-group">
        <label for="dm-target-input">Create DM</label>
        <p class="setting-desc">Start a direct message with a user by their full Matrix ID.</p>
        <div class="dm-row">
            <input id="dm-target-input" type="text" bind:value={dmTargetUserId} placeholder="@alice:etchtest.emptytincan.com" class="dm-input" />
            <button class="action-btn" on:click={() => {
                if (dmTargetUserId.trim()) {
                    sendCoreCommand({ type: 'Matrix', data: { type: 'CreateDirectMessage', data: { target_user_id: dmTargetUserId.trim() } } });
                }
            }}>Send DM</button>
        </div>
    </div>

    <div class="setting-group">
        <span class="setting-label">Display</span>
        <label class="checkbox-row">
            <input type="checkbox" bind:checked={$showRoomIds} />
            <span class="checkbox-label">Show Room IDs in chat header</span>
        </label>
    </div>

    <div class="setting-group">
        <span class="setting-label">Demo Mode</span>
        <p class="setting-desc">Populate the UI with fake channels, messages, and voice users for screenshots and mockups.</p>
        <button class="action-btn" on:click={() => { injectDemoData(); closeOverlay(); }}>Activate Demo Mode</button>
    </div>

    <div class="setting-group">
        <span class="setting-label">Encryption</span>
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

<style>
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
</style>
