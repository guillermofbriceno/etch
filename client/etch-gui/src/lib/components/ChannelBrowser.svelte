<script lang="ts">
    import { channels, activeChannelId, setActiveChannel, openConnect, usersByChannel, setUserVolume, matrixConnecting } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';
    import type { VoiceUser } from '$lib/stores/voiceState';
    import VoiceUserList from './VoiceUserList.svelte';
    import UserContextMenu from './UserContextMenu.svelte';

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
                        <svg class="channel-icon" width="16" height="16" viewBox="0 0 24 24">
                            <path fill="currentColor" d="M11.383 3.07904C11.009 2.92504 10.579 3.01004 10.293 3.29604L6 8.00204H3C2.45 8.00204 2 8.45304 2 9.00204V15.002C2 15.552 2.45 16.002 3 16.002H6L10.293 20.71C10.579 20.996 11.009 21.082 11.383 20.927C11.757 20.772 12 20.407 12 20.002V4.00204C12 3.59904 11.757 3.23204 11.383 3.07904ZM14 5.00195V7.00195C16.757 7.00195 19 9.24595 19 12.002C19 14.759 16.757 17.002 14 17.002V19.002C17.86 19.002 21 15.863 21 12.002C21 8.14295 17.86 5.00195 14 5.00195ZM14 9.00195C15.654 9.00195 17 10.349 17 12.002C17 13.657 15.654 15.002 14 15.002V13.002C14.551 13.002 15 12.553 15 12.002C15 11.451 14.551 11.002 14 11.002V9.00195Z"></path>
                        </svg>
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
                        <svg class="channel-icon" width="16" height="16" viewBox="0 0 24 24">
                            <path fill="currentColor" fill-rule="evenodd" clip-rule="evenodd" d="M5.88657 21C5.57547 21 5.3399 20.7189 5.39427 20.4126L6.00001 17H2.59511C2.28449 17 2.04905 16.7198 2.10259 16.4138L2.27759 15.4138C2.31946 15.1746 2.52722 15 2.77011 15H6.35001L7.41001 9H4.00511C3.69449 9 3.45905 8.71977 3.51259 8.41381L3.68759 7.41381C3.72946 7.17456 3.93722 7 4.18011 7H7.76001L8.39677 3.41262C8.43914 3.17391 8.64664 3 8.88907 3H9.87344C10.1845 3 10.4201 3.28107 10.3657 3.58738L9.76001 7H15.76L16.3968 3.41262C16.4391 3.17391 16.6466 3 16.8891 3H17.8734C18.1845 3 18.4201 3.28107 18.3657 3.58738L17.76 7H21.1649C21.4755 7 21.711 7.28023 21.6574 7.58619L21.4824 8.58619C21.4405 8.82544 21.2328 9 20.9899 9H17.41L16.35 15H19.7549C20.0655 15 20.301 15.2802 20.2474 15.5862L20.0724 16.5862C20.0305 16.8254 19.8228 17 19.5799 17H16L15.3632 20.5874C15.3209 20.8261 15.1134 21 14.8709 21H13.8866C13.5755 21 13.3399 20.7189 13.3943 20.4126L14 17H8.00001L7.36325 20.5874C7.32088 20.8261 7.11337 21 6.87094 21H5.88657ZM9.41045 9L8.35045 15H14.3504L15.4104 9H9.41045Z"></path>
                        </svg>
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
                            class="channel-item {$activeChannelId === channel.id ? 'active' : ''}"
                            on:click={() => setActiveChannel(channel.id)}
                        >
                            <span class="unread-slot" class:visible={channel.unread_count > 0}><span class="unread-dot"></span></span>
                            <svg class="channel-icon" width="16" height="16" viewBox="0 0 24 24">
                                <path fill="currentColor" d="M12 2C6.477 2 2 6.477 2 12s4.477 10 10 10 10-4.477 10-10S17.523 2 12 2zM8.5 9.5a1.5 1.5 0 1 1 0 3 1.5 1.5 0 0 1 0-3zm7 0a1.5 1.5 0 1 1 0 3 1.5 1.5 0 0 1 0-3zM12 18c-2.33 0-4.32-1.45-5.12-3.5h1.67c.69 1.19 1.97 2 3.45 2s2.76-.81 3.45-2h1.67c-.8 2.05-2.79 3.5-5.12 3.5z"/>
                            </svg>
                            <span class="channel-name">{channel.display_name}</span>
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

    .channel-icon { margin-right: 6px; flex-shrink: 0; }

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
