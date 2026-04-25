<script lang="ts">
    import { currentUser, isMuted, isDeafened, toggleMute, toggleDeafen, openSettings, mumbleStatus } from '$lib/stores';
</script>

<div class="user-panel">
    <div class="user-identity" on:click={() => openSettings('account')}>
        <div class="avatar-wrapper">
            {#if $currentUser.avatarUrl}
                <img src={$currentUser.avatarUrl.replace('mxc://', 'etch-media://')} alt="avatar" class="avatar" />
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
                <svg width="20" height="20" viewBox="0 0 24 24">
                    <path fill="currentColor" d="M6.7 4.7L4.7 6.7L8.8 10.8C8.9 11.2 9 11.6 9 12C9 13.7 10.3 15 12 15C12.4 15 12.8 14.9 13.2 14.8L15.3 16.9C14.4 17.6 13.3 18 12 18C8.7 18 6 15.3 6 12H4C4 16 7 19.2 11 19.9V22H13V19.9C14 19.7 14.9 19.3 15.7 18.7L17.3 20.3L19.3 18.3L6.7 4.7ZM15 12V4C15 2.3 13.7 1 12 1C10.3 1 9 2.3 9 4V7.8L15 13.8V12ZM18 12H20C20 13.5 19.6 14.9 18.8 16L17.3 14.5C17.7 13.8 18 12.9 18 12Z" />
                </svg>
            {:else}
                <svg width="20" height="20" viewBox="0 0 24 24">
                    <path fill="currentColor" d="M12 15C13.6569 15 15 13.6569 15 12V4C15 2.34315 13.6569 1 12 1C10.3431 1 9 2.34315 9 4V12C9 13.6569 10.3431 15 12 15Z" />
                    <path fill="currentColor" d="M20 12H18C18 15.3137 15.3137 18 12 18C8.68629 18 6 15.3137 6 12H4C4 16.4183 7.58172 20 12 20V22H13V19.9381C16.6966 19.5312 19.6644 16.5165 20 12Z" />
                </svg>
            {/if}
        </button>

        <button
            class="control-btn {$isDeafened ? 'danger-state' : ''}"
            on:click={toggleDeafen}
            aria-label="Toggle Deafen"
        >
            {#if $isDeafened}
                <svg width="20" height="20" viewBox="0 0 24 24">
                    <path fill="currentColor" d="M21.026 19.612L3.388 1.974L1.974 3.388L5.353 6.767C4.502 8.243 4 9.969 4 11.854V20.854C4 21.406 4.448 21.854 5 21.854H9V14.854H6V11.854C6 10.865 6.22 9.936 6.611 9.112L12.515 15.016L12.502 15.015L12 15.002V20.002C12 20.554 12.448 21.002 13 21.002H15.002L19.612 21.026L21.026 19.612ZM15 15.854V14.854H18V15.854H15ZM10.518 7.689L13.345 10.516C13.784 10.741 14.17 11.037 14.502 11.391L16.299 13.188C17.37 12.433 18 11.196 18 11.854V14.854H19.5V11.854C19.5 7.717 16.136 4.354 12 4.354C11.488 4.354 10.99 4.413 10.518 4.516V7.689Z" />
                </svg>
            {:else}
                <svg width="20" height="20" viewBox="0 0 24 24">
                    <path fill="currentColor" d="M12 3C7.031 3 3 7.031 3 12V20C3 20.552 3.447 21 4 21H8V14H5V12C5 8.14 8.14 5 12 5C15.86 5 19 8.14 19 12V14H16V21H20C20.553 21 21 20.552 21 20V12C21 7.031 16.969 3 12 3Z" />
                </svg>
            {/if}
        </button>

        <button class="control-btn" on:click={() => openSettings()} aria-label="User Settings">
            <svg width="20" height="20" viewBox="0 0 24 24">
                <path fill="currentColor" fill-rule="evenodd" clip-rule="evenodd" d="M19.738 10H22V14H19.739C19.498 14.931 19.1 15.798 18.565 16.564L20.166 18.165L17.336 20.995L15.735 19.393C14.969 19.929 14.102 20.326 13.171 20.567V23H9.17099V20.566C8.23999 20.326 7.37299 19.928 6.60699 19.393L5.00599 20.995L2.17699 18.165L3.77699 16.564C3.24199 15.798 2.84499 14.932 2.60399 14H0.362991V10H2.60399C2.84499 9.068 3.24199 8.202 3.77699 7.436L2.17699 5.835L5.00599 3.005L6.60699 4.607C7.37299 4.071 8.23999 3.674 9.17099 3.433V1H13.171V3.433C14.102 3.674 14.969 4.071 15.735 4.607L17.336 3.005L20.166 5.835L18.565 7.436C19.1 8.202 19.498 9.069 19.738 10ZM11.171 16C13.38 16 15.171 14.209 15.171 12C15.171 9.791 13.38 8 11.171 8C8.96199 8 7.17099 9.791 7.17099 12C7.17099 14.209 8.96199 16 11.171 16Z" />
            </svg>
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
