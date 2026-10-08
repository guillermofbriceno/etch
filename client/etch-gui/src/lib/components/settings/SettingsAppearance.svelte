<script lang="ts">
    import { theme, compactChat, followNewMessages, setFollowNewMessages } from '$lib/stores';
    import type { Theme } from '$lib/stores';
</script>

<div class="tab-pane">
    <h2>Appearance</h2>

    <div class="setting-group">
        <label for="theme-select">Theme</label>
        <select id="theme-select" class="hardware-select" value={$theme} on:change={(e) => theme.set(e.currentTarget.value as Theme)}>
            <option value="default">Default</option>
            <option value="oled">OLED Optimized</option>
        </select>
    </div>

    <div class="setting-group">
        <span class="section-header">Chat Display</span>
        <label class="checkbox-option">
            <input type="checkbox" bind:checked={$compactChat} />
            <span class="checkbox-label">Compact Chat</span>
        </label>
    </div>

    <div class="setting-group">
        <label class="checkbox-option">
            <input type="checkbox" checked={$followNewMessages} on:change={(e) => setFollowNewMessages(e.currentTarget.checked)} />
            <span class="checkbox-label">Switch to new messages</span>
        </label>
        <p class="setting-desc">Open a channel or direct message when a new message arrives in it. Etch stays where it is while you are writing a message.</p>
    </div>
</div>
