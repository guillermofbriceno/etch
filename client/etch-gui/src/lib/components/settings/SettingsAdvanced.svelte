<script lang="ts">
    import { sendCoreCommand } from '$lib/ipc';

    export let devMode: boolean;

    let extraMumbleArgs = '';

    function openMumbleGui() {
        sendCoreCommand({ type: 'System', data: { type: 'OpenMumbleGui', data: extraMumbleArgs } });
    }

    function restartMumble() {
        sendCoreCommand({ type: 'System', data: { type: 'RestartMumble', data: extraMumbleArgs } });
    }
</script>

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
