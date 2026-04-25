<script lang="ts">
    import { toastError } from '$lib/stores';
</script>

{#if $toastError}
    <div class="toast">
        <svg class="toast-icon" width="16" height="16" viewBox="0 0 24 24">
            <path fill="currentColor" d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-2h2v2zm0-4h-2V7h2v6z"/>
        </svg>
        <span class="toast-message">{$toastError}</span>
        <button class="toast-dismiss" on:click={() => toastError.set(null)} aria-label="Dismiss">
            <svg width="14" height="14" viewBox="0 0 24 24">
                <path fill="currentColor" d="M18.4 4L12 10.4L5.6 4L4 5.6L10.4 12L4 18.4L5.6 20L12 13.6L18.4 20L20 18.4L13.6 12L20 5.6L18.4 4Z"/>
            </svg>
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

    .toast-icon {
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
