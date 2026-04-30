<script lang="ts">
    import { errorLog } from '$lib/stores';

    let copyLabel = 'Copy All';

    async function copyErrorLog() {
        const text = $errorLog.map(e =>
            `[${e.timestamp.toLocaleTimeString()}] [${e.target}] ${e.message}`
        ).join('\n');
        await navigator.clipboard.writeText(text);
        copyLabel = 'Copied!';
        setTimeout(() => copyLabel = 'Copy All', 2000);
    }
</script>

<div class="tab-pane">
    <div class="tab-header">
        <h2>Error Log</h2>
        {#if $errorLog.length > 0}
            <button class="action-btn secondary" on:click={copyErrorLog}>{copyLabel}</button>
        {/if}
    </div>
    {#if $errorLog.length === 0}
        <p class="placeholder-text">No errors recorded.</p>
    {:else}
        <div class="error-log-list">
            {#each $errorLog as entry}
                <div class="error-entry">
                    <div class="error-meta">
                        <span class="error-timestamp">{entry.timestamp.toLocaleTimeString()}</span>
                        <span class="error-target">{entry.target}</span>
                    </div>
                    <div class="error-message">{entry.message}</div>
                </div>
            {/each}
        </div>
    {/if}
</div>

<style>
    .error-log-list {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }

    .error-entry {
        background-color: rgba(237, 66, 69, 0.08);
        border-left: 3px solid #ed4245;
        border-radius: 4px;
        padding: 10px 12px;
    }

    .error-meta {
        display: flex;
        gap: 10px;
        margin-bottom: 4px;
        font-size: 12px;
    }

    .error-timestamp { color: #72767d; }

    .error-target {
        color: #8e9297;
        font-family: monospace;
        -webkit-user-select: text;
        user-select: text;
        cursor: text;
    }

    .error-message {
        color: #dcddde;
        font-size: 14px;
        word-break: break-word;
        -webkit-user-select: text;
        user-select: text;
        cursor: text;
    }
</style>
