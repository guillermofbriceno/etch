<script module lang="ts">
    let dialogs = 0;
    // In the order they opened. The last is on top: it is drawn above the rest and owns the keyboard.
    const open: { id: number; layer: number }[] = [];
</script>

<script lang="ts">
    import { createEventDispatcher, onMount, tick } from 'svelte';

    export let title: string;
    export let dismissable = true;
    // Focus moves to the new screen's first control when this changes.
    export let screen = '';

    const dispatch = createEventDispatcher<{ dismiss: void }>();
    const id = ++dialogs;
    const titleId = `dialog-title-${id}`;
    const layer = (open[open.length - 1]?.layer ?? -1) + 1;
    const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href]';

    let backdrop: HTMLDivElement;
    let panel: HTMLDivElement;
    let shown = screen;
    let closing = false;

    $: if (panel && screen !== shown) {
        shown = screen;
        tick().then(takeFocus);
    }

    function controls(): HTMLElement[] {
        return Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE));
    }

    function isTopmost(): boolean {
        return open[open.length - 1]?.id === id;
    }

    function takeFocus() {
        if (!panel || !isTopmost()) return;
        const preferred = panel.querySelector<HTMLElement>('[data-initial-focus]:not(:disabled)');
        (preferred ?? controls()[0] ?? panel).focus();
    }

    onMount(() => {
        const opener = document.activeElement;
        open.push({ id, layer });
        takeFocus();
        return () => {
            closing = true;
            open.splice(open.findIndex((dialog) => dialog.id === id), 1);
            if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
        };
    });

    // A control that is disabled while it holds focus lets the next Tab leave the dialog.
    function containFocus(event: FocusEvent) {
        if (closing || !isTopmost()) return;
        if (event.target instanceof Node && !backdrop.contains(event.target)) takeFocus();
    }

    // In the capture phase, so Escape does not also close an overlay this sits above.
    function handleKeydown(event: KeyboardEvent) {
        if (!isTopmost()) return;
        if (event.key === 'Escape') {
            event.stopPropagation();
            if (dismissable) dispatch('dismiss');
        } else if (event.key === 'Tab') {
            const items = controls();
            const first = items[0] ?? panel;
            const last = items[items.length - 1] ?? panel;
            const active = document.activeElement;
            if (!panel.contains(active) || active === (event.shiftKey ? first : last) || active === panel) {
                event.preventDefault();
                (event.shiftKey ? last : first).focus();
            }
        }
    }
</script>

<svelte:window on:keydown|capture={handleKeydown} on:focusin={containFocus} />

<div class="dialog-backdrop" style:z-index={10000 + layer} bind:this={backdrop}>
    {#if dismissable}
        <button class="backdrop-close" tabindex="-1" on:click={() => dispatch('dismiss')} aria-label="Close dialog"></button>
    {/if}
    <div class="dialog-panel" role="dialog" aria-modal="true" aria-labelledby={titleId} tabindex="-1" bind:this={panel}>
        <h3 id={titleId}>{title}</h3>
        <slot />
    </div>
</div>

<style>
    .dialog-backdrop {
        position: fixed;
        top: var(--titlebar-height);
        left: 0;
        width: 100vw;
        height: calc(100vh - var(--titlebar-height));
        background-color: rgba(0, 0, 0, 0.7);
        display: flex;
        align-items: center;
        justify-content: center;
    }

    .backdrop-close {
        position: absolute;
        inset: 0;
        background: none;
        border: none;
        cursor: default;
    }

    .dialog-panel {
        position: relative;
        z-index: 1;
        background-color: var(--bg-tertiary);
        border-radius: 8px;
        padding: 32px;
        width: 480px;
        max-width: 90vw;
        max-height: 90%;
        overflow-y: auto;
        box-sizing: border-box;
        outline: none;
    }

    .dialog-panel h3 {
        color: var(--text-bright);
        font-size: 18px;
        font-weight: 600;
        margin: 0 0 8px;
    }

    /* Scoped under .dialog-panel so the dialogs built on this share one set of styles. */

    .dialog-panel :global(.prompt) {
        color: var(--text-secondary);
        font-size: var(--font-size-base);
        line-height: 1.4;
        margin: 0 0 12px;
        overflow-wrap: break-word;
    }

    .dialog-panel :global(.prompt strong) { color: var(--text-primary); }
    .dialog-panel :global(.prompt.warning) { color: var(--status-warning); }
    .dialog-panel :global(.prompt.danger) { color: var(--status-danger); }

    .dialog-panel :global(.actions) {
        display: flex;
        flex-wrap: wrap;
        gap: 12px;
        margin-top: 8px;
    }

    .dialog-panel :global(.prompt + .actions) { margin-top: 20px; }

    .dialog-panel :global(.text-field) {
        width: 100%;
        background-color: var(--bg-input);
        color: var(--text-primary);
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 10px;
        font-size: 16px;
        font-family: 'Inter', sans-serif;
        outline: none;
        box-sizing: border-box;
        margin: 8px 0 4px;
    }

    .dialog-panel :global(.text-field:focus) { border-color: var(--primary); }

    .dialog-panel :global(.action-btn) {
        padding: 8px 20px;
        border: none;
        border-radius: 4px;
        background-color: var(--primary);
        color: var(--text-bright);
        font-size: var(--font-size-base);
        font-family: 'Inter', sans-serif;
        font-weight: 500;
        cursor: pointer;
        transition: background-color 0.15s;
    }

    .dialog-panel :global(.action-btn:hover:not(:disabled)) { background-color: var(--primary-hover); }
    .dialog-panel :global(.action-btn:disabled) { opacity: 0.4; cursor: default; }
    .dialog-panel :global(.action-btn.secondary) { background-color: var(--bg-active); }
    .dialog-panel :global(.action-btn.secondary:hover:not(:disabled)) { background-color: rgba(255, 255, 255, 0.12); }
    .dialog-panel :global(.action-btn.danger) { background-color: var(--status-danger); }
    .dialog-panel :global(.action-btn.danger:hover:not(:disabled)) { background-color: #c93b3e; }
</style>
