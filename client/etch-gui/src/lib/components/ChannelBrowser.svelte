<script lang="ts">
    import { channels, activeChannelId, setActiveChannel, openConnect, usersByChannel, setUserVolume, matrixConnecting, hideDm } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';
    import type { VoiceUser } from '$lib/stores/voiceState';
    import VoiceUserList from './VoiceUserList.svelte';
    import UserContextMenu from './UserContextMenu.svelte';
    import Icon from './Icon.svelte';

    let dropdownOpen = false;

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
        .sort((a, b) => (a.channel_id ?? 999) - (b.channel_id ?? 999));

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

<div class="channel-browser">
    <header class="browser-header" on:click|stopPropagation={toggleDropdown}>
        <h1>Etch Server</h1>
        <span class="dropdown-indicator" class:open={dropdownOpen}>▾</span>

        {#if dropdownOpen}
            <div class="dropdown-menu">
                <button class="dropdown-item" on:click|stopPropagation={handleConnect}>Connect</button>
                <button class="dropdown-item disabled" disabled>Disconnect</button>
                <button class="dropdown-item disabled" disabled>Server Information</button>
            </div>
        {/if}
    </header>

    <div class="scroller">
        {#if $matrixConnecting && $channels.length === 0}
            <div class="connecting-indicator">
                <div class="spinner"></div>
                <span>Connecting...</span>
            </div>
        {/if}

        <div class="category">
            <h2 class="category-name">Voice Channels</h2>
            <ul class="channel-list">
                {#each voiceChannels as channel (channel.id)}
                    <li
                        class="channel-item {$activeChannelId === channel.id ? 'active' : ''}"
                        on:click={() => handleVoiceClick(channel)}
                        on:dblclick={() => handleVoiceDblClick(channel)}
                    >
                        <span class="unread-slot" class:visible={channel.unread_count > 0}><span class="unread-dot"></span></span>
                        <Icon name="volume" size={16} class="channel-icon" />
                        <span class="channel-name">{channel.display_name}</span>
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

        <div class="category">
            <h2 class="category-name">Text Channels</h2>
            <ul class="channel-list">
                {#each textChannels as channel (channel.id)}
                    <li
                        class="channel-item {$activeChannelId === channel.id ? 'active' : ''}"
                        on:click={() => setActiveChannel(channel.id)}
                    >
                        <span class="unread-slot" class:visible={channel.unread_count > 0}><span class="unread-dot"></span></span>
                        <Icon name="hash" size={16} class="channel-icon" />
                        <span class="channel-name">{channel.display_name}</span>
                    </li>
                {/each}
            </ul>
        </div>

        {#if dmChannels.length > 0}
            <div class="category">
                <h2 class="category-name">Direct Messages</h2>
                <ul class="channel-list">
                    {#each dmChannels as channel (channel.id)}
                        <li
                            class="channel-item dm-item {$activeChannelId === channel.id ? 'active' : ''}"
                            on:click={() => setActiveChannel(channel.id)}
                        >
                            <span class="unread-slot" class:visible={channel.unread_count > 0}><span class="unread-dot"></span></span>
                            <Icon name="smiley" size={16} class="channel-icon" />
                            <span class="channel-name">{channel.display_name}</span>
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
        color: #8e9297;
    }

    .browser-header {
        height: 48px;
        padding: 0 16px;
        display: flex;
        align-items: center;
        box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
        flex-shrink: 0;
        z-index: 2;
        cursor: pointer;
        position: relative;
        transition: background-color 0.15s;
    }

    .browser-header:hover { background-color: rgba(255, 255, 255, 0.04); }

    .browser-header h1 {
        font-size: 16px;
        font-weight: 700;
        color: #fff;
        margin: 0;
    }

    .dropdown-indicator {
        color: #b9bbbe;
        margin-left: 6px;
        font-size: 14px;
        transition: transform 0.15s;
    }

    .dropdown-indicator.open { transform: rotate(180deg); }

    .dropdown-menu {
        position: absolute;
        top: 100%;
        left: 8px;
        right: 8px;
        background-color: #18191c;
        border-radius: 4px;
        padding: 6px;
        box-shadow: 0 8px 16px rgba(0, 0, 0, 0.24);
        z-index: 10;
    }

    .dropdown-item {
        display: block;
        width: 100%;
        background: transparent;
        border: none;
        color: #b9bbbe;
        text-align: left;
        padding: 8px 10px;
        border-radius: 3px;
        font-size: 14px;
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: background-color 0.1s, color 0.1s;
    }

    .dropdown-item:hover:not(:disabled) { background-color: #7289da; color: #fff; }
    .dropdown-item.disabled { color: #4f545c; cursor: default; }

    .scroller {
        flex-grow: 1;
        overflow-y: scroll;
        overflow-x: hidden;
        padding: 16px 8px 16px 16px;
    }

    .scroller::-webkit-scrollbar { width: 4px; }
    .scroller::-webkit-scrollbar-track { background: transparent; }
    .scroller::-webkit-scrollbar-thumb { background-color: #202225; border-radius: 4px; }

    .category { margin-bottom: 20px; }

    .category-name {
        font-size: 12px;
        text-transform: uppercase;
        font-weight: 600;
        letter-spacing: 0.25px;
        margin: 0 0 4px 0;
        padding-left: 2px;
    }

    .channel-list { list-style: none; padding: 0; margin: 0; }

    .channel-item {
        display: flex;
        align-items: center;
        padding: 6px 8px 6px 0;
        margin-bottom: 2px;
        border-radius: 4px;
        cursor: pointer;
        transition: background-color 0.1s ease, color 0.1s ease;
    }

    .channel-item :global(.channel-icon) { margin-right: 6px; flex-shrink: 0; }

    .unread-slot {
        width: 12px;
        display: flex;
        align-items: center;
        justify-content: flex-start;
        flex-shrink: 0;
        margin-left: -8px;
    }

    .unread-slot .unread-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background-color: #5865f2;
        opacity: 0;
    }

    .unread-slot.visible .unread-dot { opacity: 1; }


    .channel-name {
        font-size: 16px;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        line-height: 20px;
    }

    .channel-item:hover { background-color: #393c43; color: #dcddde; }
    .channel-item.active { background-color: #42464d; color: #fff; }

    .hide-dm-btn {
        display: none;
        margin-left: auto;
        background: transparent;
        border: none;
        color: #8e9297;
        cursor: pointer;
        padding: 2px;
        border-radius: 3px;
        flex-shrink: 0;
        align-items: center;
    }

    .hide-dm-btn:hover { color: #fff; background-color: rgba(255, 255, 255, 0.1); }
    .dm-item:hover .hide-dm-btn { display: flex; }

    .connecting-indicator {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 12px 8px;
        color: #b9bbbe;
        font-size: 14px;
    }

    .spinner {
        width: 16px;
        height: 16px;
        border: 2px solid rgba(255, 255, 255, 0.1);
        border-top-color: #b9bbbe;
        border-radius: 50%;
        animation: spin 0.8s linear infinite;
    }

    @keyframes spin {
        to { transform: rotate(360deg); }
    }
</style>
