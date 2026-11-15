<script lang="ts">
    import { onMount } from 'svelte';
    import { EMOJI_GROUPS, isRenderable, searchEmoji, type Emoji } from '$lib/emoji';
    import { detectRenderSupport } from '$lib/emojiSupport';
    import { customScrollbar } from '$lib/scrollbar';

    export let onpick: (emoji: string) => void;
    export let onclose: () => void;

    const support = detectRenderSupport();
    const groups = EMOJI_GROUPS
        .map((g) => ({ ...g, emojis: g.emojis.filter((e) => isRenderable(e, support)) }))
        .filter((g) => g.emojis.length > 0);

    let query = '';
    let activeGroup = groups[0];
    let hovered: Emoji | null = null;
    let searchEl: HTMLInputElement;
    let gridEl: HTMLDivElement;

    $: searching = query.trim() !== '';
    $: shown = searching ? searchEmoji(query).filter((e) => isRenderable(e, support)) : activeGroup.emojis;
    $: preview = hovered ?? shown[0] ?? null;
    $: shown, gridEl && (gridEl.scrollTop = 0);

    onMount(() => searchEl.focus());

    function selectGroup(group: typeof activeGroup) {
        activeGroup = group;
        query = '';
        searchEl.focus();
    }

    function handleKeydown(event: KeyboardEvent) {
        if (event.key === 'Escape') {
            event.preventDefault();
            event.stopPropagation();
            onclose();
        } else if (event.key === 'Enter' && searching && shown.length > 0) {
            event.preventDefault();
            onpick(shown[0].emoji);
        }
    }
</script>

<div class="emoji-picker floating">
    <div class="search">
        <input
            bind:this={searchEl}
            bind:value={query}
            on:keydown={handleKeydown}
            class="search-input"
            type="search"
            placeholder="Search emoji"
            aria-label="Search emoji"
            spellcheck="false"
            autocomplete="off"
        />
    </div>

    <div class="emoji-tabs">
        {#each groups as group (group.name)}
            <button
                class="emoji-tab"
                class:active={!searching && activeGroup === group}
                title={group.name}
                aria-label={group.name}
                on:click={() => selectGroup(group)}
            >{group.icon}</button>
        {/each}
    </div>

    <div class="emoji-scroll" bind:this={gridEl} use:customScrollbar={{ width: 6, minThumbHeight: 20 }}>
        <div class="section-label">{searching ? 'Search results' : activeGroup.name}</div>
        {#if shown.length === 0}
            <div class="empty">No emoji match "{query.trim()}"</div>
        {:else}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="emoji-grid" on:mouseleave={() => hovered = null}>
                {#each shown as e (e.emoji)}
                    <button
                        class="emoji-cell"
                        aria-label={e.name}
                        on:mouseenter={() => hovered = e}
                        on:click={() => onpick(e.emoji)}
                    >{e.emoji}</button>
                {/each}
            </div>
        {/if}
    </div>

    <div class="preview">
        {#if preview}
            <span class="preview-emoji">{preview.emoji}</span>
            <span class="preview-name">{preview.name}</span>
        {/if}
    </div>
</div>

<style>
    .emoji-picker {
        width: 352px;
        height: 420px;
        display: flex;
        flex-direction: column;
        overflow: hidden;
    }

    .search { padding: 10px 10px 6px; }

    .search-input {
        width: 100%;
        box-sizing: border-box;
        background-color: var(--bg-input);
        color: var(--text-primary);
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 7px 10px;
        font-size: var(--font-size-base);
        font-family: var(--font-family);
        outline: none;
        appearance: none;
    }

    .search-input::-webkit-search-cancel-button { appearance: none; }

    .search-input:focus { border-color: var(--primary); }
    .search-input::placeholder { color: var(--text-muted); }

    .emoji-tabs {
        display: flex;
        border-bottom: 1px solid var(--border-input);
        padding: 0 6px;
    }

    .emoji-tab {
        flex: 1;
        background: none;
        border: none;
        border-bottom: 2px solid transparent;
        padding: 6px 0;
        font-family: var(--font-family-emoji);
        font-size: 18px;
        cursor: pointer;
        border-radius: 4px 4px 0 0;
        transition: background-color 0.1s;
    }

    .emoji-tab:hover { background-color: var(--bg-hover); }
    .emoji-tab.active { border-bottom-color: var(--accent); }

    .emoji-scroll {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        padding: 6px 8px 8px;
    }

    .section-label {
        padding: 4px 4px 6px;
        font-size: 12px;
        font-weight: 600;
        color: var(--text-tertiary);
    }

    .emoji-grid {
        display: grid;
        grid-template-columns: repeat(8, 1fr);
        gap: 2px;
    }

    .emoji-cell {
        width: 36px;
        height: 36px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: none;
        border: none;
        border-radius: 4px;
        font-family: var(--font-family-emoji);
        font-size: 22px;
        cursor: pointer;
        transition: background-color 0.1s;
    }

    .emoji-cell:hover { background-color: var(--bg-active); }

    .empty {
        padding: 24px 4px;
        text-align: center;
        color: var(--text-muted);
        font-size: var(--font-size-base);
    }

    .preview {
        display: flex;
        align-items: center;
        gap: 10px;
        height: 44px;
        flex-shrink: 0;
        padding: 0 12px;
        border-top: 1px solid var(--border-input);
    }

    .preview-emoji { font-family: var(--font-family-emoji); font-size: 24px; line-height: 1; }

    .preview-name {
        color: var(--text-secondary);
        font-size: 13px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
</style>
