<script lang="ts">
    import { getCurrentWindow } from '@tauri-apps/api/window';

    const appWindow = getCurrentWindow();
</script>

<div class="title-bar" on:mousedown={() => appWindow.startDragging()}>
    <span class="title">Etch</span>
    <div class="controls" on:mousedown|stopPropagation>
        <button class="control-btn" on:click={() => appWindow.minimize()} aria-label="Minimize">
            <svg width="12" height="12" viewBox="0 0 12 12">
                <rect fill="currentColor" x="1" y="5.5" width="10" height="1" />
            </svg>
        </button>
        <button class="control-btn" on:click={() => appWindow.toggleMaximize()} aria-label="Maximize">
            <svg width="12" height="12" viewBox="0 0 12 12">
                <rect fill="none" stroke="currentColor" stroke-width="1" x="1.5" y="1.5" width="9" height="9" rx="1" />
            </svg>
        </button>
        <button class="control-btn close-btn" on:click={() => appWindow.close()} aria-label="Close">
            <svg width="12" height="12" viewBox="0 0 12 12">
                <path fill="currentColor" d="M2.22 2.22a.75.75 0 011.06 0L6 4.94l2.72-2.72a.75.75 0 111.06 1.06L7.06 6l2.72 2.72a.75.75 0 11-1.06 1.06L6 7.06 3.28 9.78a.75.75 0 01-1.06-1.06L4.94 6 2.22 3.28a.75.75 0 010-1.06z" />
            </svg>
        </button>
    </div>
</div>

<style>
    .title-bar {
        display: flex;
        align-items: center;
        justify-content: space-between;
        height: var(--titlebar-height);
        padding: 0 8px 0 12px;
        user-select: none;
        flex-shrink: 0;
    }

    .title {
        font-size: 12px;
        font-weight: 600;
        color: #8e9297;
    }

    .controls {
        display: flex;
        gap: 2px;
    }

    .control-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 28px;
        height: 24px;
        background: transparent;
        border: none;
        color: #b9bbbe;
        cursor: pointer;
        border-radius: 4px;
        transition: background-color 0.1s, color 0.1s;
    }

    .control-btn:hover {
        background-color: rgba(255, 255, 255, 0.1);
        color: #fff;
    }

    .close-btn:hover {
        background-color: #ed4245;
        color: #fff;
    }
</style>
