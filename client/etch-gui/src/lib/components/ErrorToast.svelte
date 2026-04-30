<script lang="ts">
    import { toastError } from '$lib/stores';
    import Icon from './Icon.svelte';
</script>

{#if $toastError}
    <div class="toast">
        <Icon name="info" size={16} class="toast-icon" />
        <span class="toast-message">{$toastError}</span>
        <button class="toast-dismiss" on:click={() => toastError.set(null)} aria-label="Dismiss">
            <Icon name="close" size={14} />
        </button>
    </div>
{/if}

<style>
    .toast {
        position: fixed;
        top: 40px;
        left: 50%;
        transform: translateX(-50%);
        z-index: 9999;
        display: flex;
        align-items: center;
        gap: 10px;
        background-color: #2d1214;
        border: 1px solid #ed4245;
        border-radius: 6px;
        padding: 10px 14px;
        max-width: 600px;
        box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
        animation: slide-in 0.2s ease-out;
    }

    .toast :global(.toast-icon) {
        color: #ed4245;
        flex-shrink: 0;
    }

    .toast-message {
        color: #dcddde;
        font-size: 14px;
        line-height: 1.4;
        word-break: break-word;
    }

    .toast-dismiss {
        background: none;
        border: none;
        color: #72767d;
        cursor: pointer;
        padding: 2px;
        flex-shrink: 0;
        display: flex;
        align-items: center;
    }

    .toast-dismiss:hover {
        color: #dcddde;
    }

    @keyframes slide-in {
        from { opacity: 0; transform: translateX(-50%) translateY(-10px); }
        to { opacity: 1; transform: translateX(-50%) translateY(0); }
    }
</style>
