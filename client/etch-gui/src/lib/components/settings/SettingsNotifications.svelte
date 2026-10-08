<script lang="ts">
    import { desktopNotifications, setDesktopNotifications, followNewMessages, setFollowNewMessages } from '$lib/stores';
</script>

<div class="tab-pane">
    <h2>Notifications</h2>

    <div class="setting-group">
        <label class="checkbox-option">
            <input type="checkbox" checked={$desktopNotifications} on:change={(e) => setDesktopNotifications(e.currentTarget.checked)} />
            <span class="checkbox-label">Desktop notifications</span>
        </label>
        <p class="setting-desc">Show a system notification for a new message, and when someone joins or leaves your voice channel. Nothing is shown while Etch is the focused window or while you are deafened.</p>
    </div>

    <div class="setting-group">
        <label class="checkbox-option">
            <input type="checkbox" checked={$followNewMessages} on:change={(e) => setFollowNewMessages(e.currentTarget.checked)} />
            <span class="checkbox-label">Follow messages</span>
        </label>
        <p class="setting-desc">Open a channel or direct message when a new message arrives in it. Etch stays where it is while you are writing a message.</p>
    </div>
</div>
