<script lang="ts">
    export let error: string | null = null;
    export let note = '';
</script>

<div class="dialog-status">
    {#if error}
        <p class="error" role="alert">{error}</p>
    {:else if note}
        <p role="status">{note}</p>
    {/if}
</div>

<style>
    /* One line is always held open, so a note or a short reason does not move the buttons. */
    .dialog-status {
        min-height: 1.4em;
        margin: 8px 0;
        font-size: var(--font-size-base);
        line-height: 1.4;
    }

    .dialog-status p {
        margin: 0;
        color: var(--text-tertiary);
        overflow-wrap: break-word;
    }

    .dialog-status p.error { color: var(--status-danger); }
</style>
