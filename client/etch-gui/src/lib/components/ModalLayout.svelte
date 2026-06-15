<script lang="ts">
    import { closeOverlay } from '$lib/stores';
    import Icon from './Icon.svelte';
    import { customScrollbar } from '$lib/scrollbar';
</script>

<div class="modal-layout">
    <div class="modal-sidebar">
        <div class="sidebar-content">
            <slot name="sidebar" />
        </div>
    </div>

    <div class="modal-content" use:customScrollbar>
        <div class="content-container">
            <slot />
        </div>

        <div class="close-action">
            <button class="close-btn" on:click={closeOverlay} aria-label="Close">
                <Icon name="close" size={18} />
            </button>
            <span class="esc-hint">ESC</span>
        </div>
    </div>
</div>

<style>
    .modal-layout {
        display: flex;
        width: 100%;
        height: 100%;
        background-color: var(--bg-primary);
    }

    .modal-sidebar {
        flex: 1 1 auto;
        display: flex;
        justify-content: flex-end;
        background-color: var(--bg-secondary);
        padding-top: 60px;
        padding-right: 20px;
    }

    .sidebar-content { width: 218px; display: flex; flex-direction: column; flex: 1; }

    .modal-content {
        flex: 1 1 800px;
        display: flex;
        position: relative;
        background-color: var(--bg-primary);
        padding-top: 60px;
        padding-left: 40px;
        overflow-y: auto;
    }

    .content-container { width: 100%; max-width: 740px; padding-bottom: 60px; }

    .close-action {
        flex-shrink: 0;
        margin-left: 20px;
        margin-right: 20px;
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 8px;
    }

    .close-btn {
        width: 36px;
        height: 36px;
        border-radius: 50%;
        background-color: transparent;
        border: 2px solid var(--text-muted);
        color: var(--text-muted);
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        transition: background-color 0.15s, color 0.15s, border-color 0.15s;
    }

    .close-btn:hover { background-color: rgba(255, 255, 255, 0.1); color: var(--text-primary); border-color: var(--text-primary); }

    .esc-hint { color: var(--text-muted); font-size: 13px; font-weight: 600; }
</style>
