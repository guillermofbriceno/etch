<script lang="ts">
    import { createEventDispatcher } from 'svelte';
    import type { VoiceUser } from '$lib/stores/voiceState';
    import { talkingUsers } from '$lib/stores';
    import Icon from './Icon.svelte';
    import AvatarFallback from './AvatarFallback.svelte';
    import { resolveMediaUrl, getInitial } from '$lib/media';

    export let users: VoiceUser[];

    const dispatch = createEventDispatcher<{ usercontextmenu: { user: VoiceUser; event: MouseEvent } }>();

    function handleContextMenu(e: MouseEvent, user: VoiceUser) {
        e.preventDefault();
        dispatch('usercontextmenu', { user, event: e });
    }

    function ringColor(user: VoiceUser, talking: boolean): string {
        if (user.deafened) return '#ed4245';
        if (user.muted) return '#72767d';
        if (talking) return '#3ba55d';
        return '#2d2dba';
    }

    function initial(user: VoiceUser): string {
        return getInitial(user.display_name ?? user.name);
    }
</script>

<li class="voice-avatars">
    {#each users as user (user.session_id)}
        {@const src = resolveMediaUrl(user.avatar_url)}
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <span
            class="avatar-ring"
            style="border-color: {ringColor(user, $talkingUsers.has(user.session_id))}"
            title={user.display_name ?? user.name}
            on:contextmenu={(e) => handleContextMenu(e, user)}
        >
            {#if src}
                <img src={src} alt="" class="mini-avatar" />
            {:else}
                <AvatarFallback initial={initial(user)} size={18} />
            {/if}
        </span>
    {/each}
</li>
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

    /* Hidden by default; shown by container query when narrow */
    .voice-avatars {
        display: none;
        flex-wrap: wrap;
        justify-content: center;
        gap: 3px;
        padding: 2px 0;
        list-style: none;
    }

    .avatar-ring {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 18px;
        height: 18px;
        border-radius: 50%;
        border: 2px solid;
        flex-shrink: 0;
        cursor: default;
    }

    .mini-avatar {
        width: 100%;
        height: 100%;
        border-radius: 50%;
        object-fit: cover;
    }

    @container sidebar (max-width: 149px) {
        .voice-avatars { display: flex; }
        .voice-user { display: none; }
    }
</style>
