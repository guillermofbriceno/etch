<script lang="ts">
    import { createEventDispatcher } from 'svelte';
    import { userVolumes, setUserVolume, createDirectMessage, currentUser } from '$lib/stores';
    import type { VoiceUser } from '$lib/stores/voiceState';

    export let user: VoiceUser;
    export let x: number;
    export let y: number;

    const dispatch = createEventDispatcher();

    function handleMessage() {
        // Extract server domain from current user's Matrix ID (e.g. @admin:server.com → server.com)
        const parts = $currentUser.matrixId.split(':');
        const domain = parts.slice(1).join(':');
        const targetMatrixId = `@${user.name}:${domain}`;
        createDirectMessage(targetMatrixId);
        dispatch('close');
    }

    function handleVolumeInput(event: Event) {
        setUserVolume(user.session_id, parseFloat((event.target as HTMLInputElement).value));
    }

    function handleVolumeDblClick() {
        setUserVolume(user.session_id, 0);
    }
</script>

<div
    class="user-context-menu"
    style="left: {x}px; top: {y}px;"
>
    <div class="context-header">{user.display_name ?? user.name}</div>
    <button class="context-item" on:click={handleMessage}>Message</button>
    <div class="context-divider"></div>
    <div class="context-volume">
        <label class="context-volume-label">
            Volume
            <span class="context-volume-value">
                {#if ($userVolumes[user.session_id] ?? 0) === 0}
                    0 dB
                {:else if $userVolumes[user.session_id] > 0}
                    +{$userVolumes[user.session_id].toFixed(1)} dB
                {:else}
                    {$userVolumes[user.session_id].toFixed(1)} dB
                {/if}
            </span>
        </label>
        <input
            type="range"
            min="-30"
            max="30"
            step="0.1"
            value={$userVolumes[user.session_id] ?? 0}
            on:input={handleVolumeInput}
            on:dblclick={handleVolumeDblClick}
            class="volume-slider"
        />
    </div>
</div>

<style>
    .user-context-menu {
        position: fixed;
        z-index: 100;
        background-color: #18191c;
        border-radius: 4px;
        padding: 6px;
        box-shadow: 0 8px 16px rgba(0, 0, 0, 0.24);
        min-width: 180px;
    }

    .context-header {
        padding: 6px 10px;
        font-size: 13px;
        font-weight: 600;
        color: #fff;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .context-item {
        display: block;
        width: 100%;
        background: transparent;
        border: none;
        color: #b9bbbe;
        text-align: left;
        padding: 8px 10px;
        border-radius: 3px;
        font-size: 14px;
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: background-color 0.1s, color 0.1s;
    }

    .context-item:hover { background-color: #5865f2; color: #fff; }

    .context-divider {
        height: 1px;
        background-color: #2e3035;
        margin: 4px 10px;
    }

    .context-volume {
        padding: 6px 10px 8px;
    }

    .context-volume-label {
        display: flex;
        justify-content: space-between;
        align-items: center;
        font-size: 12px;
        text-transform: uppercase;
        font-weight: 600;
        letter-spacing: 0.25px;
        color: #8e9297;
        margin-bottom: 6px;
    }

    .context-volume-value {
        font-weight: 400;
        text-transform: none;
        letter-spacing: normal;
        color: #b9bbbe;
    }

    .volume-slider {
        width: 100%;
        height: 4px;
        -webkit-appearance: none;
        appearance: none;
        background: #4f545c;
        border-radius: 2px;
        outline: none;
        cursor: pointer;
    }

    .volume-slider::-webkit-slider-thumb {
        -webkit-appearance: none;
        appearance: none;
        width: 12px;
        height: 12px;
        border-radius: 50%;
        background: #fff;
        cursor: pointer;
    }

    .volume-slider::-moz-range-thumb {
        width: 12px;
        height: 12px;
        border-radius: 50%;
        background: #fff;
        border: none;
        cursor: pointer;
    }
</style>
