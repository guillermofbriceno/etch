<script lang="ts">
    import { toggleReaction, setReply, setEditing, redactMessage, cancelSend, currentUser } from '$lib/stores';
    import { onDestroy } from 'svelte';
    import Icon from './Icon.svelte';
    import EmojiPicker from './EmojiPicker.svelte';
    import type { ChatMessage } from '$lib/types';

    export let message: ChatMessage;
    export let roomId: string;

    const QUICK_EMOJIS = ['👍', '❤️', '😂', '😮', '😢', '🎉'];
    const DELETE_CONFIRM_TIMEOUT_MS = 5_000;

    $: isOwnMessage = message.sender === $currentUser.matrixId;

    let confirmingDelete = false;
    let confirmTimer: ReturnType<typeof setTimeout> | null = null;

    const PICKER_GAP = 4;
    const VIEWPORT_MARGIN = 8;

    let pickerOpen = false;
    let pickerButton: HTMLButtonElement;
    let toolbarEl: HTMLDivElement;
    let pickerEl: HTMLDivElement | null = null;

    function openPicker() {
        pickerOpen = true;
        document.addEventListener('click', closeOnOutside, true);
        window.addEventListener('scroll', closeOnScroll, true);
        window.addEventListener('resize', closePicker);
    }

    function closePicker() {
        pickerOpen = false;
        document.removeEventListener('click', closeOnOutside, true);
        window.removeEventListener('scroll', closeOnScroll, true);
        window.removeEventListener('resize', closePicker);
    }

    function closeOnOutside(event: MouseEvent) {
        const target = event.target as Node;
        if (!pickerEl?.contains(target) && !pickerButton.contains(target)) closePicker();
    }

    // The picker is fixed to the viewport, so it would drift from its message when the timeline scrolls.
    function closeOnScroll(event: Event) {
        if (!pickerEl?.contains(event.target as Node)) closePicker();
    }

    function react(emoji: string) {
        closePicker();
        toggleReaction(message.id, emoji);
    }

    /** Below the toolbar when it fits, otherwise above, kept inside the window. */
    function placeNearToolbar(node: HTMLElement) {
        const toolbar = toolbarEl.getBoundingClientRect();
        const { offsetWidth: width, offsetHeight: height } = node;
        const fitsBelow = toolbar.bottom + PICKER_GAP + height <= window.innerHeight - VIEWPORT_MARGIN;
        const top = fitsBelow ? toolbar.bottom + PICKER_GAP : toolbar.top - PICKER_GAP - height;
        const left = toolbar.right - width;
        node.style.top = `${Math.max(VIEWPORT_MARGIN, top)}px`;
        node.style.left = `${Math.min(Math.max(VIEWPORT_MARGIN, left), window.innerWidth - width - VIEWPORT_MARGIN)}px`;
    }

    onDestroy(closePicker);

    async function handleDelete() {
        if (!confirmingDelete) {
            confirmingDelete = true;
            confirmTimer = setTimeout(cancelDelete, DELETE_CONFIRM_TIMEOUT_MS);
            return;
        }
        if (!roomId) return;
        if (message.pending) {
            await cancelSend(roomId, message.pending.transaction_id);
        } else {
            await redactMessage(roomId, message.id);
        }
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
<div class="message-actions floating" class:open={pickerOpen} bind:this={toolbarEl} on:mouseleave={cancelDelete}>
    {#if confirmingDelete}
        <button class="action-btn confirm-delete" aria-label="Confirm delete" on:click={handleDelete}>
            <Icon name="trash" size={16} />
        </button>
        <button class="action-btn" aria-label="Cancel delete" on:click={cancelDelete}>
            <Icon name="close" size={16} />
        </button>
    {:else}
        <!-- Reacting, replying and editing all need the event ID the server has not given yet. -->
        {#if !message.pending}
            {#each QUICK_EMOJIS as emoji}
                <button
                    class="action-btn emoji-btn"
                    on:click={() => toggleReaction(message.id, emoji)}
                    aria-label="React with {emoji}"
                >{emoji}</button>
            {/each}
            <button
                class="action-btn"
                class:active={pickerOpen}
                aria-label="More reactions"
                bind:this={pickerButton}
                on:click={() => pickerOpen ? closePicker() : openPicker()}
            >
                <Icon name="smiley" size={16} />
            </button>
            <button class="action-btn" aria-label="Reply" on:click={() => setReply(message)}>
                <Icon name="reply" size={16} />
            </button>
        {/if}
        {#if isOwnMessage}
            {#if !message.pending}
                <button class="action-btn" aria-label="Edit" on:click={() => setEditing(message)}>
                    <Icon name="edit" size={16} />
                </button>
            {/if}
            <button class="action-btn delete-btn" aria-label="Delete" on:click={handleDelete}>
                <Icon name="trash" size={16} />
            </button>
        {/if}
    {/if}
</div>

{#if pickerOpen}
    <div class="reaction-picker" bind:this={pickerEl} use:placeNearToolbar>
        <EmojiPicker onpick={react} onclose={closePicker} />
    </div>
{/if}

<style>
    .message-actions {
        position: absolute;
        bottom: calc(100% - 8px);
        right: 16px;
        display: flex;
        gap: 1px;
        padding: 2px;
        opacity: 0;
        pointer-events: none;
    }

    /* Parent .message-block:hover reveals this via :global */
    .message-actions.open { opacity: 1; pointer-events: auto; }

    .reaction-picker {
        position: fixed;
        z-index: 30;
    }

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
        border-radius: 6px;
        transition: background-color 0.1s, color 0.1s;
    }

    .action-btn:hover, .action-btn.active { background-color: var(--bg-active); color: var(--text-bright); }

    .delete-btn:hover { color: var(--status-danger); }
    .confirm-delete { color: var(--status-danger); }

    .emoji-btn { font-family: var(--font-family-emoji); font-size: 16px; }
</style>
