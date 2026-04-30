<script lang="ts">
    import { toggleReaction, setReply } from '$lib/stores';
    import Icon from './Icon.svelte';
    import type { ChatMessage } from '$lib/types';

    export let message: ChatMessage;

    const QUICK_EMOJIS = ['👍', '❤️', '😂', '😮', '😢', '🎉'];
</script>

<div class="message-actions">
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
</div>

<style>
    .message-actions {
        position: absolute;
        top: -12px;
        right: 16px;
        display: flex;
        background-color: #2f3136;
        border: 1px solid #202225;
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
        color: #b9bbbe;
        cursor: pointer;
        border-radius: 3px;
        transition: background-color 0.1s, color 0.1s;
    }

    .action-btn:hover { background-color: #393c43; color: #fff; }

    .emoji-btn { font-size: 16px; }
</style>
