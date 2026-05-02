<script lang="ts">
    import { currentUser, isMuted, isDeafened, toggleMute, toggleDeafen, openSettings, mumbleStatus, sidebarContentCollapsed } from '$lib/stores';
    import Icon from './Icon.svelte';
    import AvatarFallback from './AvatarFallback.svelte';
    import { resolveMediaUrl, getInitial } from '$lib/media';
</script>

<div class="user-panel" class:collapsed={$sidebarContentCollapsed}>
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

        {#if !$sidebarContentCollapsed}
            <button class="control-btn" on:click={() => openSettings()} title="Settings" aria-label="User Settings">
                <Icon name="settings" size={18} />
            </button>
        {/if}
    </div>

    <button class="user-identity" class:content-hidden={$sidebarContentCollapsed} on:click={() => openSettings('account')}>
        <div class="user-text">
            <div class="username">{$currentUser.displayName ?? $currentUser.username}</div>
            <div class="discriminator">{$currentUser.matrixId.split(':')[0]}</div>
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
        color: #fff;
    }

    .user-identity {
        display: flex;
        align-items: center;
        margin-left: auto;
        padding: 4px 8px;
        border-radius: 4px;
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

    .user-identity.content-hidden {
        opacity: 0;
        width: 0;
        padding: 0;
        pointer-events: none;
    }

    .user-identity:hover { background-color: rgba(79, 84, 92, 0.32); }

    .avatar-wrapper {
        position: relative;
        width: 32px;
        height: 32px;
        margin-left: 8px;
        flex-shrink: 0;
    }

    .avatar { width: 100%; height: 100%; border-radius: 50%; background-color: #202225; object-fit: cover; }

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
        align-items: flex-end;
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

    .controls {
        display: flex;
        align-items: center;
        justify-content: center;
        margin-left: 2px;
    }

    .control-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 24px;
        height: 38px;
        background: transparent;
        border: none;
        border-radius: 4px;
        color: #b9bbbe;
        cursor: pointer;
        padding: 0;
        transition: color 0.15s ease, background-color 0.15s ease;
        line-height: 0;
    }

    .control-btn:hover { color: #dcddde; background-color: rgba(79, 84, 92, 0.4); }
    .control-btn.danger-state { color: #ed4245; }
    .control-btn.danger-state:hover { color: #ff6b6b; background-color: rgba(237, 66, 69, 0.15); }

</style>
