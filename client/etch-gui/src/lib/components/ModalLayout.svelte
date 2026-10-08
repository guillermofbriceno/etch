<script lang="ts">
    import { closeOverlay } from '$lib/stores';
    import Icon from './Icon.svelte';
    import { customScrollbar } from '$lib/scrollbar';
</script>

<div class="modal-layout">
    <div class="modal-sidebar">
        <slot name="sidebar" />
    </div>

    <div class="modal-main">
        <div class="modal-content" use:customScrollbar>
            <div class="content-container">
                <slot />
            </div>
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
    /* A dialog of bounded size: past these limits a larger window only adds backdrop around it. */
    .modal-layout {
        display: flex;
        box-sizing: border-box;
        width: 100%;
        max-width: 1120px;
        height: 100%;
        max-height: 800px;
        background-color: var(--bg-primary);
        border: 1px solid var(--border-input);
        border-radius: 10px;
        overflow: hidden;
    }

    .modal-sidebar {
        flex: 0 0 clamp(184px, 24%, 240px);
        box-sizing: border-box;
        display: flex;
        flex-direction: column;
        background-color: var(--bg-secondary);
        border-right: 1px solid var(--border-input);
        padding: 36px 12px 16px;
        overflow-y: auto;
        scrollbar-width: none;
    }

    .modal-main {
        position: relative;
        flex: 1 1 0;
        min-width: 0;
        display: flex;
    }

    .modal-content {
        flex: 1 1 0;
        min-width: 0;
        overflow-y: auto;
    }

    /* The right padding keeps the column clear of the close button, which does not scroll. */
    .content-container {
        max-width: 740px;
        padding: 36px 100px 48px 40px;
    }

    .close-action {
        position: absolute;
        top: 28px;
        right: 24px;
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 6px;
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
