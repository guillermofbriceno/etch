<script lang="ts">
    import { currentUser, isMuted, isDeafened, toggleMute, toggleDeafen, openSettings, mumbleStatus } from '$lib/stores';
    import Icon from './Icon.svelte';
</script>

<div class="user-panel">
    <div class="user-identity" on:click={() => openSettings('account')}>
        <div class="avatar-wrapper">
            {#if $currentUser.avatarUrl}
                <img src={$currentUser.avatarUrl.startsWith('mxc://') ? $currentUser.avatarUrl.replace('mxc://', 'etch-media://') : $currentUser.avatarUrl} alt="avatar" class="avatar" />
            {:else}
                <div class="avatar avatar-fallback">{($currentUser.displayName ?? $currentUser.username ?? '?').charAt(0).toUpperCase()}</div>
            {/if}
            <span class="status-dot {$mumbleStatus}"></span>
        </div>

        <div class="user-text">
            <div class="username">{$currentUser.displayName ?? $currentUser.username}</div>
            <div class="discriminator">{$currentUser.matrixId}</div>
        </div>
    </div>

    <div class="controls">
        <button
            class="control-btn {$isMuted ? 'danger-state' : ''}"
            on:click={toggleMute}
            aria-label="Toggle Mute"
        >
            {#if $isMuted}
                <Icon name="mic_muted" size={20} />
            {:else}
                <Icon name="mic" size={20} />
            {/if}
        </button>

        <button
            class="control-btn {$isDeafened ? 'danger-state' : ''}"
            on:click={toggleDeafen}
            aria-label="Toggle Deafen"
        >
            {#if $isDeafened}
                <Icon name="headphones_deafened" size={20} />
            {:else}
                <Icon name="headphones" size={20} />
            {/if}
        </button>

        <button class="control-btn" on:click={() => openSettings()} aria-label="User Settings">
            <Icon name="settings" size={20} />
        </button>
    </div>
</div>

<style>
    .user-panel {
        display: flex;
        align-items: center;
        justify-content: space-between;
        height: 100%;
        padding: 0 8px;
        background-color: transparent;
        color: #fff;
    }

    .user-identity {
        display: flex;
        align-items: center;
        padding: 4px 8px;
        border-radius: 4px;
        cursor: pointer;
        min-width: 0;
        transition: background-color 0.15s ease;
    }

    .user-identity:hover { background-color: rgba(79, 84, 92, 0.32); }

    .avatar-wrapper {
        position: relative;
        width: 32px;
        height: 32px;
        margin-right: 8px;
        flex-shrink: 0;
    }

    .avatar { width: 100%; height: 100%; border-radius: 50%; background-color: #202225; object-fit: cover; }
    .avatar-fallback {
        display: flex;
        align-items: center;
        justify-content: center;
        background-color: #5865f2;
        color: #fff;
        font-weight: 600;
        font-size: 14px;
    }

    .status-dot {
        position: absolute;
        bottom: -1px;
        right: -1px;
        width: 10px;
        height: 10px;
        border-radius: 50%;
        border: 2px solid #1a1a1a;
        background-color: #747f8d;
    }

    .status-dot.connected { background-color: #3ba55d; }
    .status-dot.connecting { background-color: #faa61a; }
    .status-dot.disconnected { background-color: #ed4245; }

    .user-text {
        display: flex;
        flex-direction: column;
        justify-content: center;
        line-height: 1.2;
        overflow: hidden;
    }

    .username {
        font-size: 14px;
        font-weight: 600;
        white-space: nowrap;
        text-overflow: ellipsis;
        overflow: hidden;
    }

    .discriminator { font-size: 12px; color: #b9bbbe; }

    .controls { display: flex; align-items: center; }

    .control-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 32px;
        background: transparent;
        border: none;
        border-radius: 4px;
        color: #b9bbbe;
        cursor: pointer;
        padding: 0;
        transition: color 0.15s ease, background-color 0.15s ease;
    }

    .control-btn:hover { background-color: rgba(79, 84, 92, 0.32); color: #dcddde; }
    .control-btn.danger-state { color: #ed4245; }
    .control-btn.danger-state:hover { color: #ed4245; }

</style>
