<script lang="ts">
    import { updateStatus, updateVersion, updateError, checkForUpdate, restartApp } from '$lib/stores';

    export let appVersion: string;
</script>

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

<style>
    .update-info { color: #b9bbbe; font-size: 14px; margin: 0 0 12px; }
    .update-error { color: #ed4245; font-size: 14px; margin: 0 0 12px; }
</style>
