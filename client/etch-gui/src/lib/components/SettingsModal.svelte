<script lang="ts">
    import { settingsTab, errorLog } from '$lib/stores';
    import { getVersion } from '@tauri-apps/api/app';

    import ModalLayout from './ModalLayout.svelte';
    import SettingsAccount from './settings/SettingsAccount.svelte';
    import SettingsAppearance from './settings/SettingsAppearance.svelte';
    import SettingsVoice from './settings/SettingsVoice.svelte';
    import SettingsUpdates from './settings/SettingsUpdates.svelte';
    import SettingsAdvanced from './settings/SettingsAdvanced.svelte';
    import SettingsDeveloper from './settings/SettingsDeveloper.svelte';
    import SettingsErrors from './settings/SettingsErrors.svelte';

    let activeTab = $settingsTab;
    let appVersion = '';
    let devMode = false;

    getVersion().then(v => appVersion = v);
</script>

<ModalLayout>
    <svelte:fragment slot="sidebar">
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
    </svelte:fragment>

    <div class="settings-form">
        {#if activeTab === 'account'}
            <SettingsAccount />
        {:else if activeTab === 'appearance'}
            <SettingsAppearance />
        {:else if activeTab === 'voice'}
            <SettingsVoice />
        {:else if activeTab === 'updates'}
            <SettingsUpdates {appVersion} />
        {:else if activeTab === 'advanced'}
            <SettingsAdvanced bind:devMode />
        {:else if activeTab === 'developer'}
            <SettingsDeveloper />
        {:else if activeTab === 'errors'}
            <SettingsErrors />
        {:else}
            <div class="tab-pane">
                <h2>{activeTab.charAt(0).toUpperCase() + activeTab.slice(1)}</h2>
                <p class="placeholder-text">Not implemented yet.</p>
            </div>
        {/if}
    </div>
</ModalLayout>

<style>
    /* ── Sidebar elements ── */

    .version-info {
        margin-top: auto;
        padding: 10px 10px 20px;
        font-size: 12px;
        color: #4f5660;
    }

    .group-header {
        font-size: 12px;
        font-weight: 700;
        color: var(--text-tertiary);
        text-transform: uppercase;
        margin: 0 0 8px 10px;
        letter-spacing: 0.2px;
    }

    .tab {
        background: transparent;
        border: none;
        color: var(--text-secondary);
        text-align: left;
        padding: 6px 10px;
        margin-bottom: 2px;
        border-radius: 4px;
        font-size: 16px;
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: background-color 0.15s, color 0.15s;
    }

    .tab:hover { background-color: var(--bg-hover); color: var(--text-primary); }
    .tab.active { background-color: var(--bg-active); color: var(--text-bright); }

    .divider { height: 1px; background-color: var(--bg-active); margin: 10px 10px 14px 10px; }

    .error-badge {
        background-color: var(--status-danger);
        color: var(--text-bright);
        font-size: 11px;
        font-weight: 700;
        padding: 1px 5px;
        border-radius: 8px;
        margin-left: 6px;
    }

    /* ── Shared form styles for tab content ── */
    /* Scoped under .settings-form so child tab components inherit these
       without redeclaring them. */

    .settings-form :global(.tab-pane h2) { color: var(--text-bright); font-size: 20px; font-weight: 600; margin-top: 0; margin-bottom: 20px; }

    .settings-form :global(.tab-header) { display: flex; align-items: center; justify-content: space-between; margin-bottom: 20px; }
    .settings-form :global(.tab-header h2) { margin: 0; }

    .settings-form :global(.section-header) {
        color: var(--text-bright);
        font-size: 13px;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.3px;
        margin: 8px 0 16px;
    }
    .settings-form :global(.section-header:not(:first-of-type)) { margin-top: 20px; }

    .settings-form :global(.placeholder-text) { color: var(--text-secondary); }

    .settings-form :global(.setting-group) { margin-bottom: 24px; display: flex; flex-direction: column; }

    .settings-form :global(.setting-group label),
    .settings-form :global(.setting-group .setting-label) {
        color: var(--text-tertiary);
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        margin-bottom: 8px;
    }

    .settings-form :global(.setting-group.disabled) { opacity: 0.4; pointer-events: none; }

    .settings-form :global(.setting-desc) { color: var(--text-tertiary); font-size: 13px; margin: 0 0 12px; }

    .settings-form :global(.divider) { height: 1px; background-color: var(--bg-active); margin: 10px 0 14px 0; }

    .settings-form :global(.action-btn) {
        background-color: var(--primary);
        color: var(--text-bright);
        border: none;
        border-radius: 4px;
        padding: 8px 16px;
        font-size: var(--font-size-base);
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: background-color 0.15s;
        align-self: flex-start;
    }

    .settings-form :global(.action-btn:hover:not(:disabled)) { background-color: var(--primary-hover); }
    .settings-form :global(.action-btn:disabled) { opacity: 0.4; cursor: default; }
    .settings-form :global(.action-btn.secondary) { background-color: var(--bg-active); }
    .settings-form :global(.action-btn.secondary:hover) { background-color: rgba(255, 255, 255, 0.12); }
    .settings-form :global(.action-btn.danger) { background-color: var(--status-danger); }
    .settings-form :global(.action-btn.danger:hover:not(:disabled)) { background-color: #c93b3e; }

    .settings-form :global(.action-row) { display: flex; gap: 8px; }

    .settings-form :global(.text-input) {
        background-color: var(--bg-input);
        color: var(--text-primary);
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 8px 10px;
        font-size: var(--font-size-base);
        font-family: 'Inter', sans-serif;
        outline: none;
    }

    .settings-form :global(.text-input:focus) { border-color: var(--primary); }
    .settings-form :global(.text-input::placeholder) { color: #4f5660; }

    .settings-form :global(.hardware-select) {
        background-color: var(--bg-input);
        color: var(--text-primary);
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 10px;
        font-size: 16px;
        font-family: 'Inter', sans-serif;
        outline: none;
        appearance: none;
        cursor: pointer;
    }

    .settings-form :global(.hardware-select:focus) { border-color: #7289da; }

    .settings-form :global(.checkbox-option) {
        display: flex;
        align-items: center;
        gap: 10px;
        cursor: pointer;
    }

    .settings-form :global(.checkbox-option input[type="checkbox"]) {
        accent-color: var(--primary);
        width: 16px;
        height: 16px;
        margin: 0;
    }

    .settings-form :global(.checkbox-label) { color: var(--text-primary); font-size: var(--font-size-base); font-weight: 500; text-transform: none; }

    .settings-form :global(.radio-group) { display: flex; flex-direction: column; gap: 2px; }

    .settings-form :global(.radio-option) {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 6px 0;
        cursor: pointer;
    }

    .settings-form :global(.radio-option input[type="radio"]) {
        accent-color: var(--primary);
        width: 16px;
        height: 16px;
        margin: 0;
        flex-shrink: 0;
    }

    .settings-form :global(.radio-content) { display: flex; flex-direction: column; gap: 1px; }
    .settings-form :global(.radio-label) { color: var(--text-primary); font-size: var(--font-size-base); font-weight: 500; }
    .settings-form :global(.radio-label), .settings-form :global(.radio-desc) { text-transform: none; }
    .settings-form :global(.radio-desc) { color: var(--text-muted); font-size: 12px; }

    .settings-form :global(.slider-container) { display: flex; align-items: center; gap: 16px; }
    .settings-form :global(.range-input) { flex-grow: 1; cursor: pointer; }
    .settings-form :global(.volume-readout) { color: var(--text-primary); font-size: var(--font-size-base); min-width: 40px; }
</style>
