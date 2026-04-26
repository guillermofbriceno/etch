<script lang="ts">
    import { currentUser } from '$lib/stores';
    import { toggleReaction } from '$lib/stores';
    import type { ChatMessage, SenderProfile } from '$lib/types';
    import DOMPurify from 'dompurify';
    import { openUrl } from '@tauri-apps/plugin-opener';
    import MediaRenderer from './MediaRenderer.svelte';
    import EmojiPicker from './EmojiPicker.svelte';

    export let msg: ChatMessage;
    export let sender: SenderProfile | null;
    export let continuation: boolean;

    function mxcToUrl(mxc: string): string {
        if (!mxc) return '';
        // mxc://server_name/media_id → etch-media://server_name/media_id
        return mxc.replace('mxc://', 'etch-media://');
    }

    function formatTimestamp(timestamp: number): string {
        return new Intl.DateTimeFormat(undefined, {
            hour: 'numeric',
            minute: '2-digit',
            hour12: true,
        }).format(new Date(timestamp));
    }

    const URL_RE = /https?:\/\/[^\s<>"')\]]+/g;

    function linkify(text: string): string {
        return text.replace(URL_RE, (url) => `<a href="${url}">${url}</a>`);
    }

    // Rewrite mxc:// URLs in sanitized HTML to use the Tauri media proxy
    DOMPurify.addHook('afterSanitizeAttributes', (node) => {
        if (node instanceof HTMLElement) {
            for (const attr of ['src', 'href']) {
                const val = node.getAttribute(attr);
                if (val?.startsWith('mxc://')) {
                    node.setAttribute(attr, val.replace('mxc://', 'etch-media://'));
                }
            }
        }
    });

    function messageBody(body: string, html_body: string | null): string {
        if (html_body) {
            return DOMPurify.sanitize(html_body, {
                ALLOWED_TAGS: [
                    'b', 'strong', 'i', 'em', 'u', 'del', 's', 'strike',
                    'code', 'pre', 'blockquote', 'br', 'p', 'span',
                    'ul', 'ol', 'li', 'a', 'img', 'h1', 'h2', 'h3',
                    'h4', 'h5', 'h6', 'hr', 'table', 'thead', 'tbody',
                    'tr', 'th', 'td', 'sup', 'sub', 'mx-reply',
                ],
                ALLOWED_ATTR: ['href', 'src', 'alt', 'title', 'class', 'data-mx-maths'],
            });
        }
        return linkify(
            body
                .replace(/&/g, '&amp;')
                .replace(/</g, '&lt;')
                .replace(/>/g, '&gt;')
                .replace(/\n/g, '<br>')
        );
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
</script>

<div class="message-block" class:continuation>
    {#if !continuation}
        <div class="avatar">
            {#if sender?.avatar_url}
                <img src={mxcToUrl(sender.avatar_url)} alt="avatar" />
            {:else}
                <div class="avatar-fallback">{msg.sender.charAt(1).toUpperCase()}</div>
            {/if}
        </div>
    {:else}
        <div class="avatar-gutter"></div>
    {/if}

    <div class="message-content">
        {#if !continuation}
            <div class="message-meta">
                <span class="sender" style="color: {usernameColor(msg.sender)}">{sender?.display_name ?? msg.sender.split(':')[0]}</span>
                <span class="timestamp">{formatTimestamp(msg.timestamp)}</span>
            </div>
        {/if}

        {#if !msg.media}
            <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
            <div class="body" on:click={handleLinkClick}>
                {@html messageBody(msg.body, msg.html_body)}
            </div>
        {/if}

        {#if msg.media}
            <MediaRenderer
                src={mxcToUrl(msg.media.mxc_url)}
                mimetype={msg.media.mimetype}
                body={msg.body}
            />
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
        background-color: rgba(114, 137, 218, 0.3);
        border-color: rgba(114, 137, 218, 0.5);
    }

    .reaction-badge.own:hover {
        background-color: rgba(114, 137, 218, 0.4);
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
        background-color: #2f3136;
    }

    .avatar img { width: 100%; height: 100%; object-fit: cover; }
    .avatar-fallback {
        width: 100%;
        height: 100%;
        display: flex;
        align-items: center;
        justify-content: center;
        background-color: #5865f2;
        color: #fff;
        font-weight: 600;
        font-size: 16px;
    }

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

    .body :global(strong) { font-weight: 600; color: #fff; }
    .body :global(em) { font-style: italic; }
    .body :global(code) {
        font-family: 'Consolas', monospace;
        background-color: #2f3136;
        padding: 2px 4px;
        border-radius: 3px;
        font-size: 14px;
    }
    .body :global(pre) {
        background-color: #2f3136;
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
</style>
