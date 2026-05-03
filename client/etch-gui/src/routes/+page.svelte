<script lang="ts">
    import TitleBar from '$lib/components/TitleBar.svelte';
    import ChannelBrowser from '$lib/components/ChannelBrowser.svelte';
    import ChatWindow from '$lib/components/ChatWindow.svelte';
    import MessageInput from '$lib/components/MessageInput.svelte';
    import UserPanel from '$lib/components/UserPanel.svelte';
    import SettingsModal from '$lib/components/SettingsModal.svelte';
    import ImageModal from '$lib/components/ImageModal.svelte';
    import ServerConnectionModal from '$lib/components/ServerConnectionModal.svelte';
    import PasswordDialog from '$lib/components/PasswordDialog.svelte';
    import ErrorToast from '$lib/components/ErrorToast.svelte';

    import { onMount, onDestroy } from 'svelte';
    import { listen, type UnlistenFn } from '@tauri-apps/api/event';
    import { activeOverlay, overlayImageUrl, closeOverlay, loadSettings, initTheme, initLayout, initStores, sidebarCollapsed } from '$lib/stores';

    let peekSuppressed = false;
    let unlistenLeave: UnlistenFn | undefined;
    let unlistenEnter: UnlistenFn | undefined;

    function handleKeydown(event: KeyboardEvent) {
        if (event.key === 'Escape' && $activeOverlay !== 'none') closeOverlay();
    }

    onMount(async () => {
        initStores();
        loadSettings();
        initTheme();
        initLayout();

        unlistenLeave = await listen('cursor-left-window', () => {
            peekSuppressed = true;
        });
        unlistenEnter = await listen('cursor-entered-window', () => {
            peekSuppressed = false;
        });
    });

    onDestroy(() => {
        unlistenLeave?.();
        unlistenEnter?.();
    });
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="app-shell">
    <TitleBar />
    <div class="app-container" class:collapsed={$sidebarCollapsed}>
    <aside class="sidebar">
        <div class="sidebar-inner" class:peek-suppressed={peekSuppressed}>
            <div class="channel-browser-wrapper">
                <ChannelBrowser />
            </div>

            <div class="user-panel-wrapper">
                <UserPanel />
            </div>
        </div>
    </aside>

    <main class="chat-area">
        <div class="chat-window-wrapper">
            <ChatWindow />
        </div>

        <div class="message-input-wrapper">
            <MessageInput />
        </div>
    </main>

    <PasswordDialog />

    {#if $activeOverlay !== 'none'}
        <div class="overlay-backdrop">
            <button class="backdrop-close" on:click={closeOverlay} aria-label="Close overlay"></button>
            {#if $activeOverlay === 'settings'}
                <div class="overlay-content">
                    <SettingsModal />
                </div>
            {:else if $activeOverlay === 'image' && $overlayImageUrl}
                <ImageModal url={$overlayImageUrl} />
            {:else if $activeOverlay === 'connect'}
                <div class="overlay-content">
                    <ServerConnectionModal />
                </div>
            {/if}
        </div>
    {/if}
    </div>
</div>

<ErrorToast />

<style>
    .app-shell {
        display: flex;
        flex-direction: column;
        height: 100vh;
        width: 100vw;
    }

    .app-container {
        display: grid;
        grid-template-columns: 240px 1fr;
        transition: grid-template-columns 100ms ease;
        flex: 1;
        min-height: 0;
    }

    .app-container.collapsed {
        grid-template-columns: 76px 1fr;
    }

    .sidebar {
        display: flex;
        flex-direction: column;
        background-color: rgba(255, 255, 255, 0.0);
        min-height: 0;
        overflow: hidden;
    }

    .app-container.collapsed .sidebar {
        overflow: visible;
        position: relative;
    }

    .sidebar-inner {
        display: flex;
        flex-direction: column;
        gap: 10px;
        margin: 10px 0 10px 10px;
        min-height: 0;
        flex: 1;
        container-type: inline-size;
        container-name: sidebar;
    }

    .app-container.collapsed .sidebar-inner {
        position: absolute;
        top: 0;
        left: 0;
        bottom: 0;
        width: calc(100% - 10px);
        margin: 10px 0 10px 10px;
        box-sizing: border-box;
        background: var(--bg-base);
        z-index: 10;
        transition: width 150ms ease, box-shadow 150ms ease;
    }

    .app-container.collapsed .sidebar-inner:hover,
    .app-container.collapsed .sidebar-inner:has(:focus-visible) {
        width: 230px;
        z-index: 10;
        box-shadow: 4px 0 12px rgba(0, 0, 0, 0.4);
    }

    .app-container.collapsed .sidebar-inner.peek-suppressed {
        pointer-events: none;
    }

    .channel-browser-wrapper {
        flex-grow: 1;
        overflow-y: hidden;
        border-radius: 10px;
        background-color: var(--bg-panel);
        border: var(--border-panel);
        min-height: 0;
    }

    .user-panel-wrapper {
        flex-shrink: 0;
        height: 56px;
        border-radius: 10px;
        background-color: var(--bg-panel);
        border: var(--border-panel);
    }

    .chat-area {
        display: flex;
        flex-direction: column;
        background-color: rgba(0, 0, 0, 0.0);
        min-height: 0;
        min-width: 0;
    }

    .chat-window-wrapper {
        flex-grow: 1;
        overflow-y: hidden;
        display: flex;
        flex-direction: column;
        min-height: 0;
    }

    .message-input-wrapper {
        flex-shrink: 0;
        background-color: var(--bg-panel);
        border: var(--border-panel);
        border-radius: 10px;
        margin-bottom: 10px;
        margin-left: 10px;
        margin-right: 10px;
    }

    .overlay-backdrop {
        position: fixed;
        top: var(--titlebar-height);
        left: 0;
        width: 100vw;
        height: calc(100vh - var(--titlebar-height));
        background-color: rgba(0, 0, 0, 0.85);
        z-index: 9999;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 48px;
        box-sizing: border-box;
    }

    .backdrop-close {
        position: absolute;
        inset: 0;
        background: none;
        border: none;
        cursor: default;
    }

    .overlay-content {
        position: relative;
        z-index: 1;
        width: 100%;
        height: 100%;
        display: flex;
        align-items: center;
        justify-content: center;
        pointer-events: none;
    }

    .overlay-content > :global(*) {
        pointer-events: auto;
    }
</style>
