<script lang="ts">
    import { currentUser, compactChat } from '$lib/stores';
    import { toggleReaction } from '$lib/stores';
    import type { ChatMessage, SenderProfile } from '$lib/types';
    import DOMPurify from 'dompurify';
    import { openUrl } from '@tauri-apps/plugin-opener';
    import { hljs } from '$lib/highlight';
    import { markdownToHtml } from '$lib/markdown';
    import { HTML_BODY_SANITIZE } from '$lib/sanitize';
    import { replaceInTextNodes } from '$lib/dom';
    import { resolveMediaUrl, getInitial } from '$lib/media';
    import MediaRenderer from './MediaRenderer.svelte';
    import AvatarFallback from './AvatarFallback.svelte';
    import EmojiPicker from './EmojiPicker.svelte';

    export let msg: ChatMessage;
    export let sender: SenderProfile | null;
    export let continuation: boolean;

    const COLLAPSE_THRESHOLD = 300;
    let bodyEl: HTMLElement;
    let needsCollapse = false;
    let collapsed = true;

    const timeFormatter = new Intl.DateTimeFormat(undefined, {
        hour: 'numeric',
        minute: '2-digit',
        hour12: true,
    });

    function formatTimestamp(timestamp: number): string {
        return timeFormatter.format(new Date(timestamp));
    }

    function messageBody(body: string, html_body: string | null): string {
        if (html_body) {
            return DOMPurify.sanitize(html_body, HTML_BODY_SANITIZE) as string;
        }
        return markdownToHtml(body);
    }

    function usernameColor(userId: string): string {
        // FNV-1a hash — much better distribution than simple multiply-add
        let hash = 0x811c9dc5;
        for (let i = 0; i < userId.length; i++) {
            hash ^= userId.charCodeAt(i);
            hash = Math.imul(hash, 0x01000193);
        }
        const hue = ((hash >>> 0) % 360);
        return `hsl(${hue}, 70%, 65%)`;
    }

    function handleLinkClick(e: MouseEvent) {
        const anchor = (e.target as HTMLElement).closest('a');
        if (!anchor) return;
        const href = anchor.getAttribute('href');
        if (href && /^https?:\/\//.test(href)) {
            e.preventDefault();
            openUrl(href);
        }
    }

    function hasReactions(reactions: Record<string, string[]>): boolean {
        return Object.keys(reactions).length > 0;
    }

    function mentionsSelf(body: string, html_body: string | null): boolean {
        const selfId = $currentUser.matrixId;
        if (!selfId) return false;
        const text = html_body ?? body;
        return text.includes(selfId);
    }

    function processBody(node: HTMLElement, params: { htmlBody: string | null; selfId: string }) {
        function run({ htmlBody, selfId }: typeof params) {
            // Only highlight code in server-rendered HTML; markdownToHtml already highlights
            if (htmlBody) {
                node.querySelectorAll('pre code:not(.hljs)').forEach((el) => {
                    hljs.highlightElement(el as HTMLElement);
                });
            }
            applyMentionStyling(node, selfId);
        }
        run(params);
        return { update: run };
    }

    function collapseWatch(node: HTMLElement, _content: string) {
        function measure() {
            needsCollapse = node.scrollHeight > COLLAPSE_THRESHOLD;
        }

        requestAnimationFrame(measure);

        const ro = new ResizeObserver(measure);
        ro.observe(node);

        return {
            update() {
                collapsed = true;
                requestAnimationFrame(measure);
            },
            destroy() {
                ro.disconnect();
            },
        };
    }

    function toggleCollapse() {
        collapsed = !collapsed;
    }

    function applyMentionStyling(container: HTMLElement, selfId: string): void {
        // Rewrite matrix.to anchor tags to styled mention spans
        container.querySelectorAll<HTMLAnchorElement>('a[href*="matrix.to/#/@"]').forEach((a) => {
            const href = a.getAttribute('href') ?? '';
            const match = href.match(/matrix\.to\/#\/(@[^"&\s]+)/);
            if (!match) return;
            const userId = decodeURIComponent(match[1]);
            const span = document.createElement('span');
            span.className = userId === selfId ? 'mention mention-self' : 'mention';
            span.textContent = `@${a.textContent}`;
            a.replaceWith(span);
        });

        // Walk text nodes for plain @user:server mentions
        const mentionRe = /@([a-zA-Z0-9._=\-/]+:[a-zA-Z0-9.\-]+)/g;
        replaceInTextNodes(container, mentionRe, (match) => {
            const fullId = match[0];
            const localpart = match[1].split(':')[0];
            const span = document.createElement('span');
            span.className = fullId === selfId ? 'mention mention-self' : 'mention';
            span.textContent = `@${localpart}`;
            return span;
        });
    }
</script>

<div class="message-block" class:continuation class:compact={$compactChat} class:mentioned={mentionsSelf(msg.body, msg.html_body)}>
    {#if !continuation}
        <div class={$compactChat ? 'compact-avatar' : 'avatar'}>
            {#if sender?.avatar_url}
                <img src={resolveMediaUrl(sender.avatar_url)} alt="avatar" />
            {:else}
                <AvatarFallback initial={getInitial(msg.sender)} size={$compactChat ? 20 : 40} fontSize={$compactChat ? 10 : 16} />
            {/if}
        </div>
    {:else}
        <div class={$compactChat ? 'compact-gutter' : 'avatar-gutter'}></div>
    {/if}

    <div class="message-content">
        {#if !continuation}
            <div class="message-meta">
                <span class="sender" style="color: {usernameColor(msg.sender)}">{sender?.display_name ?? msg.sender.split(':')[0]}</span>
                <span class="timestamp">{formatTimestamp(msg.timestamp)}</span>
            </div>
        {/if}

        {#if !msg.media}
            <div class="body-wrapper" class:collapsed={needsCollapse && collapsed}>
                <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
                <div
                    class="body"
                    bind:this={bodyEl}
                    use:processBody={{ htmlBody: msg.html_body, selfId: $currentUser.matrixId }}
                    use:collapseWatch={msg.body + (msg.html_body ?? '')}
                    on:click={handleLinkClick}
                >
                    {@html messageBody(msg.body, msg.html_body)}
                </div>
            </div>
            {#if needsCollapse}
                <button class="collapse-toggle" on:click={toggleCollapse}>
                    {collapsed ? 'See more' : 'See less'}
                </button>
            {/if}
        {/if}

        {#if msg.media}
            {@const mediaSrc = resolveMediaUrl(msg.media.mxc_url)}
            {#if mediaSrc}
                <MediaRenderer
                    src={mediaSrc}
                    mimetype={msg.media.mimetype}
                    body={msg.body}
                />
            {/if}
        {/if}

        {#if hasReactions(msg.reactions)}
            <div class="reaction-badges">
                {#each Object.entries(msg.reactions) as [emoji, senders]}
                    <button
                        class="reaction-badge {senders.includes($currentUser.matrixId) ? 'own' : ''}"
                        on:click={() => toggleReaction(msg.id, emoji)}
                        aria-label="{emoji} {senders.length}"
                    >
                        <span class="reaction-emoji">{emoji}</span>
                        <span class="reaction-count">{senders.length}</span>
                    </button>
                {/each}
            </div>
        {/if}
    </div>

    <EmojiPicker message={msg} />
</div>

<style>
    .message-block {
        position: relative;
        display: flex;
        padding: 4px 16px;
        margin-top: 16px;
    }

    .message-block.continuation { margin-top: 0; padding-top: 1px; padding-bottom: 1px; }
    .message-block:hover { background-color: rgba(4, 4, 5, 0.07); }
    .message-block.mentioned {
        background-color: color-mix(in srgb, var(--accent) 8%, transparent);
        border-left: 3px solid var(--accent);
        padding-left: 13px;
    }
    .message-block.mentioned:hover { background-color: color-mix(in srgb, var(--accent) 12%, transparent); }
    .avatar-gutter { width: 40px; margin-right: 16px; flex-shrink: 0; }

    /* Reveal EmojiPicker (message-actions) on hover */
    .message-block:hover > :global(.message-actions) {
        opacity: 1;
        pointer-events: auto;
    }

    /* --- Reaction badges --- */
    .reaction-badges {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
        margin-top: 4px;
    }

    .reaction-badge {
        display: flex;
        align-items: center;
        gap: 4px;
        padding: 2px 6px;
        background-color: rgba(79, 84, 92, 0.3);
        border: 1px solid transparent;
        border-radius: 4px;
        cursor: pointer;
        font-size: 14px;
        color: #b9bbbe;
        transition: background-color 0.1s, border-color 0.1s;
    }

    .reaction-badge:hover {
        background-color: rgba(79, 84, 92, 0.5);
        border-color: rgba(255, 255, 255, 0.1);
    }

    .reaction-badge.own {
        background-color: color-mix(in srgb, var(--accent) 30%, transparent);
        border-color: color-mix(in srgb, var(--accent) 50%, transparent);
    }

    .reaction-badge.own:hover {
        background-color: color-mix(in srgb, var(--accent) 40%, transparent);
    }

    .reaction-emoji { font-size: 16px; line-height: 1; }
    .reaction-count { font-size: 12px; font-weight: 500; }

    /* --- Message content --- */
    .avatar {
        width: 40px;
        height: 40px;
        border-radius: 50%;
        overflow: hidden;
        margin-right: 16px;
        flex-shrink: 0;
        background-color: var(--bg-inset);
    }

    .avatar img { width: 100%; height: 100%; object-fit: cover; }
    .message-content { flex-grow: 1; min-width: 0; }

    .message-meta { margin-bottom: 4px; line-height: 1.2; }

    .sender { font-weight: 500; margin-right: 8px; }

    .timestamp { font-size: 12px; color: #72767d; }

    .body {
        color: #dcddde;
        line-height: 1.5;
        font-size: 15px;
        overflow-wrap: anywhere;
    }

    .body :global(p) { margin: 0; }
    .body :global(p + p) { margin-top: 4px; }
    .body :global(strong) { font-weight: 600; color: #fff; }
    .body :global(em) { font-style: italic; }
    .body :global(code) {
        font-family: 'Consolas', monospace;
        background-color: var(--bg-inset);
        padding: 2px 4px;
        border-radius: 3px;
        font-size: 14px;
    }
    .body :global(pre) {
        background-color: var(--bg-inset);
        padding: 8px;
        border-radius: 4px;
        border: 1px solid #202225;
        overflow-x: auto;
    }
    .body :global(pre code) { background-color: transparent; padding: 0; }
    .body :global(blockquote) {
        border-left: 3px solid #4f545c;
        padding: 2px 12px;
        margin: 4px 0;
        color: #a3a6aa;
    }
    .body :global(a) { color: #00aff4; text-decoration: none; }
    .body :global(a:hover) { text-decoration: underline; }
    .body :global(ul), .body :global(ol) { padding-left: 24px; margin: 4px 0; }
    .body :global(li) { margin: 2px 0; }
    .body :global(img) { max-width: 400px; max-height: 300px; border-radius: 4px; cursor: pointer; }
    .body :global(h1), .body :global(h2), .body :global(h3) { color: #fff; margin: 8px 0 4px; }
    .body :global(hr) { border: none; border-top: 1px solid #4f545c; margin: 8px 0; }
    .body :global(del), .body :global(s) { text-decoration: line-through; color: #a3a6aa; }
    .body :global(mx-reply) { display: none; }
    .body :global(.mention) {
        background-color: color-mix(in srgb, var(--accent) 15%, transparent);
        color: var(--accent);
        padding: 0 2px;
        border-radius: 3px;
        font-weight: 500;
    }
    .body :global(.mention-self) {
        background-color: color-mix(in srgb, var(--accent) 30%, transparent);
        color: #dee0fc;
    }

    /* --- Collapsible message body --- */
    .body-wrapper {
        position: relative;
    }

    .body-wrapper.collapsed {
        max-height: 300px;
        overflow: hidden;
        -webkit-mask-image: linear-gradient(to bottom, black calc(100% - 48px), transparent);
        mask-image: linear-gradient(to bottom, black calc(100% - 48px), transparent);
    }

    .collapse-toggle {
        display: inline-block;
        background: none;
        border: none;
        color: var(--accent);
        font-size: 13px;
        font-weight: 500;
        cursor: pointer;
        padding: 4px 0;
        margin: 0;
        font-family: inherit;
    }

    .collapse-toggle:hover {
        color: var(--accent-hover);
        text-decoration: underline;
    }

    /* --- Compact mode --- */
    .compact { padding: 0px 16px; margin-top: 1px; }
    .compact.continuation { margin-top: 0; padding-top: 0px; padding-bottom: 0px; }

    .compact-avatar {
        width: 20px;
        height: 20px;
        border-radius: 50%;
        overflow: hidden;
        margin-right: 6px;
        margin-top: 1px;
        flex-shrink: 0;
        background-color: var(--bg-inset);
    }

    .compact-avatar img { width: 100%; height: 100%; object-fit: cover; }
    .compact-gutter { width: 20px; margin-right: 6px; flex-shrink: 0; }

    .compact .message-meta { display: inline; margin-bottom: 0; }
    .compact .sender { font-size: 13px; margin-right: 4px; display: inline; }
    .compact .timestamp { font-size: 11px; margin-right: 6px; display: inline; }

    .compact .body-wrapper { display: inline; }
    .compact .body { font-size: 13px; line-height: 1.4; display: inline; }
    .compact .body :global(p:first-child) { display: inline; }
    .compact .body :global(code) { font-size: 12px; }

    .compact .reaction-badges { margin-top: 2px; }
    .compact .reaction-badge { font-size: 12px; }
    .compact .reaction-emoji { font-size: 14px; }
    .compact .reaction-count { font-size: 11px; }
</style>
