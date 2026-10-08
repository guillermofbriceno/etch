<script lang="ts">
    import { currentUser, isMuted, isDeafened, toggleMute, toggleDeafen, openSettings, mumbleStatus, matrixStatus } from '$lib/stores';
    import Icon from './Icon.svelte';
    import AvatarFallback from './AvatarFallback.svelte';
    import { resolveMediaUrl, getInitial } from '$lib/media';
</script>

<div class="user-panel">
    <div class="controls">
        <button
            class="control-btn {$isMuted ? 'danger-state' : ''}"
            on:click={toggleMute}
            title={$isMuted ? 'Unmute' : 'Mute'}
            aria-label="Toggle Mute"
        >
            {#if $isMuted}
                <Icon name="mic_muted" size={18} />
            {:else}
                <Icon name="mic" size={18} />
            {/if}
        </button>

        <button
            class="control-btn {$isDeafened ? 'danger-state' : ''}"
            on:click={toggleDeafen}
            title={$isDeafened ? 'Undeafen' : 'Deafen'}
            aria-label="Toggle Deafen"
        >
            {#if $isDeafened}
                <Icon name="headphones_deafened" size={18} />
            {:else}
                <Icon name="headphones" size={18} />
            {/if}
        </button>

        <button class="control-btn settings-btn" on:click={() => openSettings()} title="Settings" aria-label="User Settings">
            <Icon name="settings" size={18} />
        </button>
    </div>

    <button class="user-identity" on:click={() => openSettings('account')}>
        <div class="user-text">
            {#if $currentUser.matrixId}
                <div class="username">{$currentUser.displayName ?? $currentUser.username}</div>
                <div class="discriminator">{$currentUser.matrixId.split(':')[0]}</div>
            {:else if $matrixStatus === 'disconnected'}
                <div class="discriminator">Offline</div>
            {/if}
        </div>

        <div class="avatar-wrapper">
            {#if $currentUser.avatarUrl}
                <img src={resolveMediaUrl($currentUser.avatarUrl)} alt="avatar" class="avatar" />
            {:else}
                <AvatarFallback initial={getInitial($currentUser.displayName ?? $currentUser.username)} size={32} fontSize={14} />
            {/if}
            <span class="status-dot {$mumbleStatus}"></span>
        </div>
    </button>
</div>

<style>
    .user-panel {
        display: flex;
        align-items: center;
        height: 100%;
        background-color: transparent;
        color: var(--text-bright);
    }

    .user-identity {
        display: flex;
        align-items: center;
        margin-left: auto;
        margin-right: 4px;
        padding: 4px 6px 4px 8px;
        border-radius: 6px;
        cursor: pointer;
        min-width: 0;
        transition: background-color 0.15s ease, opacity 75ms ease, width 75ms ease;
        background: none;
        border: none;
        color: inherit;
        font: inherit;
        text-align: right;
        overflow: hidden;
    }

    .user-identity:hover { background-color: var(--bg-hover); }

    .avatar-wrapper {
        position: relative;
        width: 32px;
        height: 32px;
        margin-left: 8px;
        flex-shrink: 0;
    }

    .avatar { width: 100%; height: 100%; border-radius: 50%; background-color: var(--bg-inset); object-fit: cover; }

    .status-dot {
        position: absolute;
        bottom: -1px;
        right: -1px;
        width: 10px;
        height: 10px;
        border-radius: 50%;
        border: 2px solid var(--bg-primary);
        background-color: #747f8d;
    }

    .status-dot.connected { background-color: var(--status-success); }
    .status-dot.connecting { background-color: var(--status-warning); }
    .status-dot.disconnected { background-color: var(--status-danger); }

    .user-text {
        display: flex;
        flex-direction: column;
        align-items: flex-end;
        justify-content: center;
        line-height: 1.2;
        overflow: hidden;
    }

    .username {
        font-size: var(--font-size-base);
        font-weight: 600;
        white-space: nowrap;
        text-overflow: ellipsis;
        overflow: hidden;
    }

    .discriminator { font-size: 12px; color: var(--text-secondary); white-space: nowrap; }

    .controls {
        display: flex;
        align-items: center;
        justify-content: center;
        margin-left: 8px;
    }

    .control-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 28px;
        height: 32px;
        background: transparent;
        border: none;
        border-radius: 6px;
        color: var(--text-secondary);
        cursor: pointer;
        padding: 0;
        transition: color 0.15s ease, background-color 0.15s ease;
        line-height: 0;
    }

    .control-btn:hover { color: var(--text-primary); background-color: var(--bg-hover); }
    .control-btn.danger-state { color: var(--status-danger); }
    .control-btn.danger-state:hover { color: #ff6b6b; background-color: rgba(237, 66, 69, 0.15); }

    @container sidebar (max-width: 149px) {
        .settings-btn { display: none; }
        .user-identity {
            opacity: 0;
            width: 0;
            padding: 0;
            pointer-events: none;
        }
    }
</style>
