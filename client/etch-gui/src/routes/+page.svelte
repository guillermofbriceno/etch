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

    import { onMount } from 'svelte';
    import { activeOverlay, overlayImageUrl, closeOverlay, loadSettings, initTheme } from '$lib/stores';

    onMount(() => {
        loadSettings();
        initTheme();
    });
</script>

<div class="app-shell">
    <TitleBar />
    <div class="app-container">
    <aside class="sidebar">
        <div class="channel-browser-wrapper">
            <ChannelBrowser />
        </div>

        <div class="user-panel-wrapper">
            <UserPanel />
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
        <div class="overlay-backdrop" on:click={closeOverlay}>
            {#if $activeOverlay === 'settings'}
                <div class="overlay-content" on:click|stopPropagation>
                    <SettingsModal />
                </div>
            {:else if $activeOverlay === 'image' && $overlayImageUrl}
                <ImageModal url={$overlayImageUrl} />
            {:else if $activeOverlay === 'connect'}
                <div class="overlay-content" on:click|stopPropagation>
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
        flex: 1;
        min-height: 0;
    }

    .sidebar {
        display: flex;
        flex-direction: column;
        background-color: rgba(255, 255, 255, 0.0);
        min-height: 0;
        overflow: hidden;
    }

    .channel-browser-wrapper {
        flex-grow: 1;
        overflow-y: hidden;
        border-radius: 10px;
        margin-bottom: 10px;
        margin-left: 10px;
        margin-top: 10px;
        background-color: var(--bg-panel);
        border: var(--border-panel);
        min-height: 0;
    }

    .user-panel-wrapper {
        flex-shrink: 0;
        height: 52px;
        border-radius: 10px;
        margin-bottom: 10px;
        margin-left: 10px;
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

    .overlay-content {
        width: 100%;
        height: 100%;
        display: flex;
        align-items: center;
        justify-content: center;
        pointer-events: auto;
    }
</style>
