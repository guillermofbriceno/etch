<script lang="ts">
    import { channels, activeChannelId, setActiveChannel, openConnect, usersByChannel, setUserVolume, matrixStatus, matrixConnecting, connectingBookmark, hideDm, dmLastActivity, sidebarCollapsed, sidebarTransitioning, toggleSidebar } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';
    import { resolveMediaUrl, getInitial } from '$lib/media';
    import type { VoiceUser } from '$lib/stores/voiceState';
    import VoiceUserList from './VoiceUserList.svelte';
    import AvatarFallback from './AvatarFallback.svelte';
    import UserContextMenu from './UserContextMenu.svelte';
    import Icon from './Icon.svelte';
    import { customScrollbar } from '$lib/scrollbar';
    import { onMount, onDestroy } from 'svelte';

    let dropdownOpen = false;

    $: serverName = ($matrixStatus !== 'disconnected' && $connectingBookmark?.label) || 'Etch Server';

    // Close dropdown when the sidebar is toggled to collapsed.
    let unsubCollapsed: (() => void) | undefined;
    onMount(() => {
        let initialized = false;
        unsubCollapsed = sidebarCollapsed.subscribe(v => {
            if (!initialized) { initialized = true; return; }
            if (v) dropdownOpen = false;
        });
    });
    onDestroy(() => unsubCollapsed?.());

    // User context menu state
    let contextUser: VoiceUser | null = null;
    let contextX = 0;
    let contextY = 0;

    function handleUserContextMenu(e: CustomEvent<{ user: VoiceUser; event: MouseEvent }>) {
        const { user, event } = e.detail;
        contextUser = user;
        contextX = event.clientX;
        contextY = event.clientY;
    }

    function closeContextMenu() {
        contextUser = null;
    }

    $: voiceChannels = $channels
        .filter(c => c.etch_room_type === 'Voice')
        .sort((a, b) => (a.channel_id ?? 999) - (b.channel_id ?? 999));
    $: textChannels  = $channels
        .filter(c => c.etch_room_type === 'Text')
        .sort((a, b) => (a.channel_id ?? 999) - (b.channel_id ?? 999));
    $: dmChannels    = $channels
        .filter(c => c.etch_room_type === 'Dm')
        .sort((a, b) => ($dmLastActivity[b.id] ?? 0) - ($dmLastActivity[a.id] ?? 0));

    function toggleDropdown() {
        dropdownOpen = !dropdownOpen;
    }

    function handleClickOutside(event: MouseEvent) {
        if (dropdownOpen) {
            const target = event.target as HTMLElement;
            if (!target.closest('.browser-header')) {
                dropdownOpen = false;
            }
        }
        if (contextUser) {
            const target = event.target as HTMLElement;
            if (!target.closest('.user-context-menu')) {
                closeContextMenu();
            }
        }
    }

    function handleConnect() {
        dropdownOpen = false;
        openConnect();
    }

    async function joinVoiceChannel(channelId: number | null) {
        if (channelId == null) return;
        await sendCoreCommand({ type: 'Mumble', data: { type: 'SwitchChannel', data: channelId } });
    }

    function handleVoiceClick(channel: import('$lib/types').RoomInfo) {
        setActiveChannel(channel.id);
    }

    function handleVoiceDblClick(channel: import('$lib/types').RoomInfo) {
        joinVoiceChannel(channel.channel_id);
    }
</script>

<svelte:window on:click={handleClickOutside} />

<div class="channel-browser" class:transitioning={$sidebarTransitioning}>
    <header class="browser-header">
        <button
            class="collapse-btn"
            on:click|stopPropagation={toggleSidebar}
            aria-label={$sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        >
            <Icon name={$sidebarCollapsed ? 'sidebar_expand' : 'sidebar_collapse'} size={18} />
        </button>

        <button
            class="header-toggle"
            on:click|stopPropagation={toggleDropdown}
            aria-expanded={dropdownOpen}
        >
            <h1>{serverName}</h1>
            <span class="dropdown-indicator" class:open={dropdownOpen}><Icon name="chevron_down" size={16} /></span>
        </button>

        {#if dropdownOpen}
            <div class="dropdown-menu floating">
                <button class="dropdown-item" on:click|stopPropagation={handleConnect}>Connect</button>
                <button class="dropdown-item disabled" disabled>Disconnect</button>
                <button class="dropdown-item disabled" disabled>Server Information</button>
            </div>
        {/if}
    </header>

    <div class="scroller" use:customScrollbar={{ width: 4 }}>
        {#if $matrixConnecting && $channels.length === 0}
            <div class="connecting-indicator">
                <div class="spinner"></div>
                <span>Connecting...</span>
            </div>
        {/if}

        {#if voiceChannels.length > 0}
            <div class="category">
                <h2 class="category-name">Voice Channels</h2>
                <ul class="channel-list">
                    {#each voiceChannels as channel (channel.id)}
                        <li class="channel-item" class:active={$activeChannelId === channel.id} class:unread={channel.unread_count > 0}>
                            <button class="channel-btn"
                                on:click={() => handleVoiceClick(channel)}
                                on:dblclick={() => handleVoiceDblClick(channel)}
                                title={channel.display_name}
                            >
                                <Icon name="volume" size={16} class="channel-icon" />
                                <span class="channel-name full">{channel.display_name}</span>
                                <span class="channel-name abbr">{getInitial(channel.display_name)}</span>
                            </button>
                        </li>
                        {#if channel.channel_id != null && $usersByChannel.has(channel.channel_id)}
                            <VoiceUserList
                                users={$usersByChannel.get(channel.channel_id) ?? []}
                                on:usercontextmenu={handleUserContextMenu}
                            />
                        {/if}
                    {/each}
                </ul>
            </div>
        {/if}

        {#if textChannels.length > 0}
            <div class="category">
                <h2 class="category-name">Text Channels</h2>
                <ul class="channel-list">
                    {#each textChannels as channel (channel.id)}
                        <li class="channel-item" class:active={$activeChannelId === channel.id} class:unread={channel.unread_count > 0}>
                            <button class="channel-btn"
                                on:click={() => setActiveChannel(channel.id)}
                                title={channel.display_name}
                            >
                                <Icon name="hash" size={16} class="channel-icon" />
                                <span class="channel-name full">{channel.display_name}</span>
                                <span class="channel-name abbr">{getInitial(channel.display_name)}</span>
                            </button>
                        </li>
                    {/each}
                </ul>
            </div>
        {/if}

        {#if dmChannels.length > 0}
            <div class="category">
                <h2 class="category-name">Direct Messages</h2>
                <ul class="channel-list">
                    {#each dmChannels as channel (channel.id)}
                        <li class="channel-item dm-item" class:active={$activeChannelId === channel.id} class:unread={channel.unread_count > 0}>
                            <button class="channel-btn"
                                on:click={() => setActiveChannel(channel.id)}
                                title={channel.display_name}
                            >
                                    {#if channel.avatar_url}
                                    <img
                                        src={resolveMediaUrl(channel.avatar_url)}
                                        alt=""
                                        class="dm-avatar"
                                    />
                                {:else}
                                    <AvatarFallback initial={getInitial(channel.display_name)} size={20} />
                                {/if}
                                <span class="channel-name full">{channel.display_name}</span>
                            </button>
                            <button
                                class="hide-dm-btn"
                                on:click|stopPropagation={() => hideDm(channel.id)}
                                title="Hide conversation"
                            >
                                <Icon name="hide_dm" size={14} />
                            </button>
                        </li>
                    {/each}
                </ul>
            </div>
        {/if}
    </div>

    {#if contextUser}
        <UserContextMenu user={contextUser} x={contextX} y={contextY} on:close={closeContextMenu} />
    {/if}
</div>

<style>
    .channel-browser {
        display: flex;
        flex-direction: column;
        height: 100%;
        background-color: transparent;
        color: var(--text-tertiary);
        --gutter: 12px;
    }

    .browser-header {
        box-sizing: border-box;
        height: 48px;
        display: flex;
        align-items: center;
        gap: 2px;
        padding: 0 var(--gutter);
        border-bottom: 1px solid var(--border-input);
        flex-shrink: 0;
        z-index: 2;
        position: relative;
    }

    .collapse-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 32px;
        flex-shrink: 0;
        background: none;
        border: none;
        border-radius: 6px;
        color: var(--text-secondary);
        cursor: pointer;
        transition: background-color 0.15s, color 0.15s;
    }

    .collapse-btn:hover { background-color: var(--bg-hover); color: var(--text-primary); }

    .header-toggle {
        display: flex;
        align-items: center;
        gap: 4px;
        flex: 1;
        min-width: 0;
        height: 32px;
        padding: 0 8px;
        background: none;
        border: none;
        border-radius: 6px;
        color: inherit;
        font: inherit;
        cursor: pointer;
        transition: background-color 0.15s, opacity 150ms ease;
        overflow: hidden;
    }

    .header-toggle:hover, .header-toggle[aria-expanded="true"] { background-color: var(--bg-hover); }

    .header-toggle h1 {
        font-size: var(--font-size-channel);
        font-weight: 700;
        color: var(--text-bright);
        margin: 0;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .dropdown-indicator {
        display: flex;
        flex-shrink: 0;
        color: var(--text-secondary);
        transition: transform 0.15s;
    }

    .dropdown-indicator.open { transform: rotate(180deg); }

    .dropdown-menu {
        position: absolute;
        top: calc(100% + 4px);
        left: var(--gutter);
        right: var(--gutter);
        padding: 4px;
        z-index: 10;
    }

    .dropdown-item {
        display: block;
        width: 100%;
        background: transparent;
        border: none;
        color: var(--text-secondary);
        text-align: left;
        padding: 7px 10px;
        border-radius: 4px;
        font-size: var(--font-size-base);
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: background-color 0.1s, color 0.1s;
    }

    .dropdown-item:hover:not(:disabled) { background-color: var(--bg-active); color: var(--text-bright); }
    .dropdown-item.disabled { color: var(--text-muted); opacity: 0.6; cursor: default; }

    .scroller {
        flex-grow: 1;
        overflow-y: auto;
        overflow-x: hidden;
        padding: 16px var(--gutter);
    }

    .category { margin-bottom: 20px; }
    .category:last-child { margin-bottom: 0; }

    .category-name {
        font-size: 12px;
        text-transform: uppercase;
        font-weight: 700;
        letter-spacing: 0.2px;
        margin: 0 0 6px 0;
        padding-left: 8px;
        transition: opacity 150ms ease;
        overflow: hidden;
        white-space: nowrap;
    }

    .channel-list { list-style: none; padding: 0; margin: 0; }

    .channel-item {
        position: relative;
        display: flex;
        align-items: center;
        margin-bottom: 2px;
        border-radius: 4px;
        cursor: pointer;
        transition: background-color 0.1s ease, color 0.1s ease;
    }

    .channel-btn {
        display: flex;
        align-items: center;
        gap: 6px;
        flex: 1;
        min-width: 0;
        padding: var(--channel-item-padding);
        background: none;
        border: none;
        color: inherit;
        font: inherit;
        cursor: inherit;
        text-align: left;
    }

    .channel-btn :global(.channel-icon) { flex-shrink: 0; }

    /* The dot sits in the gutter, so a row's content starts at the same place with or without it. */
    .channel-item.unread::before {
        content: '';
        position: absolute;
        left: calc(var(--gutter) / -2 - 3px);
        top: 50%;
        width: 6px;
        height: 6px;
        margin-top: -3px;
        border-radius: 50%;
        background-color: var(--primary);
    }

    .channel-name {
        font-size: var(--font-size-channel);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        line-height: 20px;
        transition: opacity 150ms ease;
    }

    /* Toggle animation: fade text while container-query swap happens */
    .transitioning .channel-name { opacity: 0; }
    .transitioning .category-name { opacity: 0; }

    .channel-item.unread { color: var(--text-primary); }
    .channel-item:hover { background-color: var(--bg-hover); color: var(--text-primary); }
    .channel-item.active { background-color: var(--bg-active); color: var(--text-bright); }

    /* Default: show full names, hide abbreviations */
    .channel-name.abbr { display: none; }

    .hide-dm-btn {
        display: none;
        margin: 0 6px 0 auto;
        background: transparent;
        border: none;
        color: var(--text-tertiary);
        cursor: pointer;
        padding: 3px;
        border-radius: 4px;
        flex-shrink: 0;
        align-items: center;
    }

    .hide-dm-btn:hover { color: var(--text-bright); background-color: var(--bg-active); }
    .dm-item:hover .hide-dm-btn { display: flex; }

    .dm-avatar {
        width: 20px;
        height: 20px;
        border-radius: 50%;
        flex-shrink: 0;
        object-fit: cover;
    }

    .connecting-indicator {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 4px 8px 12px;
        color: var(--text-secondary);
        font-size: var(--font-size-base);
    }

    .spinner {
        width: 16px;
        height: 16px;
        border: 2px solid rgba(255, 255, 255, 0.1);
        border-top-color: var(--text-secondary);
        border-radius: 50%;
        animation: spin 0.8s linear infinite;
    }

    @keyframes spin {
        to { transform: rotate(360deg); }
    }

    /* Narrow container: collapsed sidebar layout */
    @container sidebar (max-width: 149px) {
        .channel-browser { --gutter: 8px; }
        .browser-header { justify-content: center; gap: 0; }
        .header-toggle {
            flex: 0 0 0;
            opacity: 0;
            padding: 0;
            pointer-events: none;
        }
        .category-name {
            opacity: 0;
            pointer-events: none;
        }
        .scroller { overflow-y: auto; scrollbar-width: none; }
        .category + .category { border-top: 1px solid var(--border-input); }
        .channel-btn { justify-content: center; gap: 4px; padding: 6px 0; }
        .channel-name.full { display: none; }
        .channel-name.abbr { display: inline; }
        .hide-dm-btn { display: none !important; }
        .dropdown-menu { display: none; }
        .connecting-indicator { justify-content: center; padding: 4px 0 12px; }
        .connecting-indicator span { display: none; }
    }
</style>
