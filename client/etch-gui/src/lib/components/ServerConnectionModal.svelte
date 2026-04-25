<script lang="ts">
    import { closeOverlay, serverBookmarks, selectedBookmarkId, addBookmark, updateBookmark, removeBookmark, connectToServer } from '$lib/stores';
    import type { ServerBookmark } from '$lib/types';

    let editLabel = '';
    let editAddress = '';
    let editPort = 8448;
    let editUsername = '';
    let editAutoConnect = false;
    let editMumbleHost = '';
    let editMumblePort = '';
    let editMumbleUsername = '';
    let editMumblePassword = '';

    let showAdvanced = false;
    let statusMessage = '';

    $: selected = $selectedBookmarkId
        ? $serverBookmarks.find(b => b.id === $selectedBookmarkId) ?? null
        : null;

    $: if (selected) {
        editLabel = selected.label;
        editAddress = selected.address;
        editPort = selected.port;
        editUsername = selected.username;
        editAutoConnect = selected.auto_connect;
        editMumbleHost = selected.mumble_host ?? '';
        editMumblePort = selected.mumble_port != null ? String(selected.mumble_port) : '';
        editMumbleUsername = selected.mumble_username ?? '';
        editMumblePassword = selected.mumble_password ?? '';
        showAdvanced = !!(selected.mumble_host || selected.mumble_port || selected.mumble_username || selected.mumble_password);
    }

    function handleSave() {
        if (!selected) return;
        updateBookmark(selected.id, {
            label: editLabel,
            address: editAddress,
            port: editPort,
            username: editUsername,
            auto_connect: editAutoConnect,
            mumble_host: editMumbleHost || null,
            mumble_port: editMumblePort ? Number(editMumblePort) : null,
            mumble_username: editMumbleUsername || null,
            mumble_password: editMumblePassword || null,
        });
    }

    async function handleConnect() {
        if (!selected) return;
        statusMessage = 'Connecting...';
        try {
            await connectToServer(selected);
        } catch (e) {
            statusMessage = `Connection failed: ${e}`;
        }
    }

    function handleRemove() {
        if (!selected) return;
        removeBookmark(selected.id);
    }

    function handleKeydown(event: KeyboardEvent) {
        if (event.key === 'Escape') closeOverlay();
    }
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="connect-layout">
    <div class="connect-sidebar">
        <div class="sidebar-content">
            <h3 class="group-header">Server Bookmarks</h3>
            <ul class="bookmark-list">
                {#each $serverBookmarks as bookmark (bookmark.id)}
                    <li>
                        <button
                            class="bookmark-item {$selectedBookmarkId === bookmark.id ? 'active' : ''}"
                            on:click={() => selectedBookmarkId.set(bookmark.id)}
                        >
                            {bookmark.label || 'Untitled'}
                        </button>
                    </li>
                {/each}
            </ul>
            <button class="add-btn" on:click={addBookmark}>+ Add New</button>
        </div>
    </div>

    <div class="connect-content">
        <div class="content-container">
            {#if selected}
                <div class="editor-pane">
                    <h2>Edit Server</h2>

                    <div class="field-group">
                        <label for="srv-label">Label</label>
                        <input id="srv-label" type="text" bind:value={editLabel} placeholder="My Server" />
                    </div>

                    <div class="field-row">
                        <div class="field-group field-grow">
                            <label for="srv-address">Address</label>
                            <input id="srv-address" type="text" bind:value={editAddress} placeholder="matrix.example.com" />
                        </div>
                        <div class="field-group field-port">
                            <label for="srv-port">Port</label>
                            <input id="srv-port" type="number" bind:value={editPort} min="1" max="65535" />
                        </div>
                    </div>

                    <div class="field-group">
                        <label for="srv-username">Username</label>
                        <input id="srv-username" type="text" bind:value={editUsername} placeholder="@user" />
                    </div>

                    <div class="field-group checkbox-group">
                        <label>
                            <input type="checkbox" bind:checked={editAutoConnect} />
                            Auto-connect on startup
                        </label>
                    </div>

                    <button class="advanced-toggle" on:click={() => showAdvanced = !showAdvanced}>
                        <span class="advanced-arrow" class:open={showAdvanced}>&#9654;</span>
                        Advanced
                    </button>

                    {#if showAdvanced}
                        <div class="advanced-section">
                            <div class="field-row">
                                <div class="field-group field-grow">
                                    <label for="srv-mumble-host">Mumble Host</label>
                                    <input id="srv-mumble-host" type="text" bind:value={editMumbleHost} placeholder="Same as address" />
                                </div>
                                <div class="field-group field-port">
                                    <label for="srv-mumble-port">Port</label>
                                    <input id="srv-mumble-port" type="text" bind:value={editMumblePort} placeholder="64738" />
                                </div>
                            </div>

                            <div class="field-group">
                                <label for="srv-mumble-username">Mumble Username</label>
                                <input id="srv-mumble-username" type="text" bind:value={editMumbleUsername} placeholder="Same as username" />
                            </div>

                            <div class="field-group">
                                <label for="srv-mumble-password">Mumble Password</label>
                                <input id="srv-mumble-password" type="password" bind:value={editMumblePassword} placeholder="None" />
                            </div>
                        </div>
                    {/if}

                    <div class="action-bar">
                        <button class="action-btn save-btn" on:click={handleSave}>Save</button>
                        <button class="action-btn connect-btn" on:click={handleConnect}>Connect</button>
                        <button class="action-btn remove-btn" on:click={handleRemove}>Remove</button>
                    </div>

                    {#if statusMessage}
                        <p class="status-message">{statusMessage}</p>
                    {/if}
                </div>
            {:else}
                <div class="empty-state">
                    <p>Select a server bookmark or add a new one.</p>
                </div>
            {/if}
        </div>

        <div class="close-action">
            <button class="close-btn" on:click={closeOverlay} aria-label="Close">
                <svg width="18" height="18" viewBox="0 0 24 24">
                    <path fill="currentColor" d="M18.4 4L12 10.4L5.6 4L4 5.6L10.4 12L4 18.4L5.6 20L12 13.6L18.4 20L20 18.4L13.6 12L20 5.6L18.4 4Z" />
                </svg>
            </button>
            <span class="esc-hint">ESC</span>
        </div>
    </div>
</div>

<style>
    .connect-layout {
        display: flex;
        width: 100%;
        height: 100%;
        background-color: #121212;
    }

    .connect-sidebar {
        flex: 1 1 auto;
        display: flex;
        justify-content: flex-end;
        background-color: #0a0a0a;
        padding-top: 60px;
        padding-right: 20px;
    }

    .sidebar-content { width: 218px; display: flex; flex-direction: column; }

    .group-header {
        font-size: 12px;
        font-weight: 700;
        color: #8e9297;
        text-transform: uppercase;
        margin: 0 0 8px 10px;
        letter-spacing: 0.2px;
    }

    .bookmark-list {
        list-style: none;
        padding: 0;
        margin: 0;
        flex: 1;
        overflow-y: auto;
    }

    .bookmark-item {
        display: block;
        width: 100%;
        background: transparent;
        border: none;
        color: #b9bbbe;
        text-align: left;
        padding: 6px 10px;
        margin-bottom: 2px;
        border-radius: 4px;
        font-size: 16px;
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: background-color 0.15s, color 0.15s;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .bookmark-item:hover { background-color: rgba(255, 255, 255, 0.06); color: #dcddde; }
    .bookmark-item.active { background-color: rgba(255, 255, 255, 0.08); color: #fff; }

    .add-btn {
        background: transparent;
        border: 1px dashed rgba(255, 255, 255, 0.15);
        color: #b9bbbe;
        padding: 8px 10px;
        margin-top: 8px;
        border-radius: 4px;
        font-size: 14px;
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: background-color 0.15s, color 0.15s, border-color 0.15s;
    }

    .add-btn:hover { background-color: rgba(255, 255, 255, 0.06); color: #dcddde; border-color: rgba(255, 255, 255, 0.25); }

    .connect-content {
        flex: 1 1 800px;
        display: flex;
        position: relative;
        background-color: #121212;
        padding-top: 60px;
        padding-left: 40px;
        overflow-y: auto;
    }

    .connect-content::-webkit-scrollbar { width: 6px; }
    .connect-content::-webkit-scrollbar-track { background: transparent; }
    .connect-content::-webkit-scrollbar-thumb { background-color: #202225; border-radius: 3px; }

    .content-container { width: 100%; max-width: 740px; padding-bottom: 60px; }

    .editor-pane h2 { color: #fff; font-size: 20px; font-weight: 600; margin-top: 0; margin-bottom: 20px; }

    .field-group { margin-bottom: 20px; display: flex; flex-direction: column; }

    .field-group label {
        color: #8e9297;
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        margin-bottom: 8px;
    }

    .field-group input[type="text"],
    .field-group input[type="number"] {
        background-color: rgba(255, 255, 255, 0.06);
        color: #dcddde;
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 4px;
        padding: 10px;
        font-size: 16px;
        font-family: 'Inter', sans-serif;
        outline: none;
    }

    .field-group input[type="text"]:focus,
    .field-group input[type="number"]:focus {
        border-color: #7289da;
    }

    .field-row { display: flex; gap: 16px; }
    .field-grow { flex: 1; }
    .field-port { width: 100px; }

    .checkbox-group label {
        display: flex;
        align-items: center;
        gap: 8px;
        color: #b9bbbe;
        font-size: 14px;
        text-transform: none;
        font-weight: 400;
        cursor: pointer;
    }

    .checkbox-group input[type="checkbox"] {
        accent-color: #7289da;
        width: 16px;
        height: 16px;
        cursor: pointer;
    }

    .advanced-toggle {
        display: flex;
        align-items: center;
        gap: 6px;
        background: transparent;
        border: none;
        color: #8e9297;
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        cursor: pointer;
        padding: 0;
        margin-bottom: 16px;
        font-family: 'Inter', sans-serif;
        letter-spacing: 0.2px;
        transition: color 0.15s;
    }

    .advanced-toggle:hover { color: #dcddde; }

    .advanced-arrow {
        display: inline-block;
        font-size: 10px;
        transition: transform 0.15s;
    }

    .advanced-arrow.open { transform: rotate(90deg); }

    .advanced-section { margin-bottom: 4px; }

    .advanced-section .field-group input[type="text"],
    .advanced-section .field-group input[type="password"] {
        background-color: rgba(255, 255, 255, 0.06);
        color: #dcddde;
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 4px;
        padding: 10px;
        font-size: 16px;
        font-family: 'Inter', sans-serif;
        outline: none;
    }

    .advanced-section .field-group input[type="text"]:focus,
    .advanced-section .field-group input[type="password"]:focus {
        border-color: #7289da;
    }

    .action-bar { display: flex; gap: 12px; margin-top: 12px; }

    .action-btn {
        padding: 8px 20px;
        border: none;
        border-radius: 4px;
        font-size: 14px;
        font-family: 'Inter', sans-serif;
        font-weight: 500;
        cursor: pointer;
        transition: background-color 0.15s;
    }

    .save-btn { background-color: #7289da; color: #fff; }
    .save-btn:hover { background-color: #677bc4; }

    .connect-btn { background-color: #43b581; color: #fff; }
    .connect-btn:hover { background-color: #3ca374; }

    .remove-btn { background-color: #ed4245; color: #fff; }
    .remove-btn:hover { background-color: #d63031; }

    .empty-state { color: #72767d; padding-top: 40px; font-size: 16px; }

    .close-action {
        flex-shrink: 0;
        margin-left: 20px;
        margin-right: 20px;
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 8px;
    }

    .close-btn {
        width: 36px;
        height: 36px;
        border-radius: 50%;
        background-color: transparent;
        border: 2px solid #72767d;
        color: #72767d;
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        transition: background-color 0.15s, color 0.15s, border-color 0.15s;
    }

    .close-btn:hover { background-color: rgba(255, 255, 255, 0.1); color: #dcddde; border-color: #dcddde; }

    .esc-hint { color: #72767d; font-size: 13px; font-weight: 600; }

    .status-message {
        color: #b9bbbe;
        font-size: 14px;
        margin-top: 16px;
    }

</style>
