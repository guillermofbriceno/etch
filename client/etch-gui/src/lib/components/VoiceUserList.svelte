<script lang="ts">
    import { createEventDispatcher } from 'svelte';
    import type { VoiceUser } from '$lib/stores/voiceState';

    export let users: VoiceUser[];

    const dispatch = createEventDispatcher<{ usercontextmenu: { user: VoiceUser; event: MouseEvent } }>();

    function handleContextMenu(e: MouseEvent, user: VoiceUser) {
        e.preventDefault();
        dispatch('usercontextmenu', { user, event: e });
    }
</script>

{#each users as user (user.session_id)}
    <li class="voice-user" on:contextmenu={(e) => handleContextMenu(e, user)}>
        <span class="voice-user-name">{user.display_name ?? user.name}</span>
        <div class="voice-status-icons">
            {#if user.deafened}
                <svg class="voice-status-icon" width="14" height="14" viewBox="0 0 24 24">
                    <path fill="#72767d" d="M12 2a4 4 0 0 1 4 4v5a4 4 0 0 1-8 0V6a4 4 0 0 1 4-4Zm-1 17.93A7 7 0 0 1 5 13h2a5 5 0 0 0 10 0h2a7 7 0 0 1-6 6.93V22h-2v-2.07Z"/>
                    <line x1="3" y1="21" x2="21" y2="3" stroke="#ed4245" stroke-width="2.5" stroke-linecap="round"/>
                </svg>
                <svg class="voice-status-icon" width="14" height="14" viewBox="0 0 24 24">
                    <path fill="#72767d" d="M12 2C8.69 2 6 4.69 6 8v4c0 1.1-.9 2-2 2v2h4.28c.35 1.72 1.86 3 3.72 3s3.37-1.28 3.72-3H20v-2c-1.1 0-2-.9-2-2V8c0-3.31-2.69-6-6-6Z"/>
                    <line x1="3" y1="21" x2="21" y2="3" stroke="#ed4245" stroke-width="2.5" stroke-linecap="round"/>
                </svg>
            {:else if user.muted}
                <svg class="voice-status-icon" width="14" height="14" viewBox="0 0 24 24">
                    <path fill="#72767d" d="M12 2a4 4 0 0 1 4 4v5a4 4 0 0 1-8 0V6a4 4 0 0 1 4-4Zm-1 17.93A7 7 0 0 1 5 13h2a5 5 0 0 0 10 0h2a7 7 0 0 1-6 6.93V22h-2v-2.07Z"/>
                    <line x1="3" y1="21" x2="21" y2="3" stroke="#ed4245" stroke-width="2.5" stroke-linecap="round"/>
                </svg>
            {:else if user.talking}
                <svg class="voice-status-icon" width="12" height="12" viewBox="0 0 24 24">
                    <circle cx="12" cy="12" r="6" fill="#3ba55d"/>
                </svg>
            {:else}
                <svg class="voice-status-icon" width="12" height="12" viewBox="0 0 24 24">
                    <circle cx="12" cy="12" r="5" fill="none" stroke="#72767d" stroke-width="2"/>
                </svg>
            {/if}
        </div>
    </li>
{/each}

<style>
    .voice-user {
        display: flex;
        align-items: center;
        padding: 3px 8px 3px 32px;
        margin-bottom: 1px;
        color: #b9bbbe;
        list-style: none;
    }

    .voice-user-name {
        font-size: 14px;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .voice-status-icons {
        display: flex;
        align-items: center;
        gap: 2px;
        margin-left: auto;
        flex-shrink: 0;
    }

    .voice-status-icon { color: #72767d; }
</style>
