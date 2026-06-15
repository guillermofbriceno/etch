<script lang="ts">
    import { toggleReaction, setReply, setEditing, redactMessage, currentUser } from '$lib/stores';
    import Icon from './Icon.svelte';
    import type { ChatMessage } from '$lib/types';

    export let message: ChatMessage;
    export let roomId: string;

    const QUICK_EMOJIS = ['👍', '❤️', '😂', '😮', '😢', '🎉'];
    const DELETE_CONFIRM_TIMEOUT_MS = 5_000;

    $: isOwnMessage = message.sender === $currentUser.matrixId;

    let confirmingDelete = false;
    let confirmTimer: ReturnType<typeof setTimeout> | null = null;

    async function handleDelete() {
        if (!confirmingDelete) {
            confirmingDelete = true;
            confirmTimer = setTimeout(cancelDelete, DELETE_CONFIRM_TIMEOUT_MS);
            return;
        }
        if (!roomId) return;
        await redactMessage(roomId, message.id);
        cancelDelete();
    }

    function cancelDelete() {
        confirmingDelete = false;
        if (confirmTimer) {
            clearTimeout(confirmTimer);
            confirmTimer = null;
        }
    }
</script>

<!-- svelte-ignore a11y-no-static-element-interactions -->
<div class="message-actions" on:mouseleave={cancelDelete}>
    {#if confirmingDelete}
        <button class="action-btn confirm-delete" aria-label="Confirm delete" on:click={handleDelete}>
            <Icon name="trash" size={16} />
        </button>
        <button class="action-btn" aria-label="Cancel delete" on:click={cancelDelete}>
            <Icon name="close" size={16} />
        </button>
    {:else}
        {#each QUICK_EMOJIS as emoji}
            <button
                class="action-btn emoji-btn"
                on:click={() => toggleReaction(message.id, emoji)}
                aria-label="React with {emoji}"
            >{emoji}</button>
        {/each}
        <button class="action-btn" aria-label="Reply" on:click={() => setReply(message)}>
            <Icon name="reply" size={16} />
        </button>
        {#if isOwnMessage}
            <button class="action-btn" aria-label="Edit" on:click={() => setEditing(message)}>
                <Icon name="edit" size={16} />
            </button>
            <button class="action-btn delete-btn" aria-label="Delete" on:click={handleDelete}>
                <Icon name="trash" size={16} />
            </button>
        {/if}
    {/if}
</div>

<style>
    .message-actions {
        position: absolute;
        bottom: calc(100% - 8px);
        right: 26px;
        display: flex;
        background-color: #2f3136;
        border: 1px solid var(--border-subtle);
        border-radius: 4px;
        opacity: 0;
        pointer-events: none;
        transition: opacity 0.1s ease;
    }

    /* Parent .message-block:hover reveals this via :global */

    .action-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 30px;
        height: 30px;
        background: transparent;
        border: none;
        color: var(--text-secondary);
        cursor: pointer;
        border-radius: 3px;
        transition: background-color 0.1s, color 0.1s;
    }

    .action-btn:hover { background-color: #393c43; color: var(--text-bright); }

    .delete-btn:hover { color: #ed4245; }
    .confirm-delete { color: #ed4245; }

    .emoji-btn { font-size: 16px; }
</style>
