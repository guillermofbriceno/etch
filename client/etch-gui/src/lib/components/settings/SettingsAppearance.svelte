<script lang="ts">
    import { theme } from '$lib/stores';
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
</div>
