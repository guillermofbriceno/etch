<script lang="ts">
    import { createEventDispatcher } from 'svelte';
    import type { VoiceUser } from '$lib/stores/voiceState';
    import { talkingUsers } from '$lib/stores';
    import Icon from './Icon.svelte';

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
                <Icon name="voice_mic_muted" size={14} class="voice-status-icon" />
                <Icon name="voice_headphones_muted" size={14} class="voice-status-icon" />
            {:else if user.muted}
                <Icon name="voice_mic_muted" size={14} class="voice-status-icon" />
            {:else if $talkingUsers.has(user.session_id)}
                <Icon name="voice_talking" size={12} class="voice-status-icon" />
            {:else}
                <Icon name="voice_silent" size={12} class="voice-status-icon" />
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

    .voice-status-icons :global(.voice-status-icon) { color: #72767d; }
</style>
