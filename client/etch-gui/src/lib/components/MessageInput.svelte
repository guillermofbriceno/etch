<script lang="ts">
    import { onMount, onDestroy } from 'svelte';
    import { get } from 'svelte/store';
    import { open } from '@tauri-apps/plugin-dialog';
    import { remove } from '@tauri-apps/plugin-fs';
    import { invoke } from '@tauri-apps/api/core';
    import { sendMessage, activeChannelId, activeChannel, activeWindow, replyingTo, clearReply } from '$lib/stores';
    import { composeHtml, insertMentionLinks } from '$lib/markdown';

    let messageText = '';
    let showEmojiPicker = false;
    let textareaEl: HTMLTextAreaElement;
    let pickerAnchorEl: HTMLDivElement;
    let pendingAttachment: { path: string; temp: boolean } | null = null;
    let composeLock = false;

    // Tab-completion state
    let tabPrefix = '';
    let tabMatches: { displayName: string; matrixId: string }[] = [];
    let tabIndex = -1;
    let tabStart = -1;
    let tabEnd = -1;
    // Tracks completed mentions in the current message: displayName -> matrixId
    const mentionMap = new Map<string, string>();

    // Clear mention and tab-completion state when switching channels
    $: $activeChannelId, (() => {
        mentionMap.clear();
        tabMatches = [];
        tabIndex = -1;
    })();

    // Derived reactively from timeline so we don't rebuild on every Tab press
    $: roomUsers = (() => {
        const seen = new Map<string, string>();
        for (const entry of $activeWindow.entries) {
            const kind = entry.kind;
            if (typeof kind === 'object' && 'Message' in kind) {
                const matrixId = kind.Message.sender;
                if (!seen.has(matrixId)) {
                    const name = entry.sender?.display_name ?? matrixId.slice(1).split(':')[0];
                    seen.set(matrixId, name);
                }
            }
        }
        return Array.from(seen, ([matrixId, displayName]) => ({ displayName, matrixId }));
    })();

    function handleTabCompletion() {
        const cursor = textareaEl.selectionStart;

        if (tabIndex >= 0 && tabMatches.length > 0) {
            // Cycle to next match
            tabIndex = (tabIndex + 1) % tabMatches.length;
        } else {
            // Start new completion: find the @word behind the cursor
            const before = messageText.slice(0, cursor);
            const match = before.match(/@(\S*)$/);
            if (!match) return;

            tabPrefix = match[1].toLowerCase();
            tabStart = cursor - match[0].length;
            tabEnd = cursor;

            tabMatches = roomUsers.filter((u) =>
                u.displayName.toLowerCase().startsWith(tabPrefix) ||
                u.matrixId.slice(1).split(':')[0].toLowerCase().startsWith(tabPrefix),
            );
            if (tabMatches.length === 0) return;
            tabIndex = 0;
        }

        const user = tabMatches[tabIndex];
        mentionMap.set(user.displayName, user.matrixId);
        const replacement = `@${user.displayName} `;
        messageText = messageText.slice(0, tabStart) + replacement + messageText.slice(tabEnd);
        tabEnd = tabStart + replacement.length;

        requestAnimationFrame(() => {
            textareaEl.selectionStart = tabEnd;
            textareaEl.selectionEnd = tabEnd;
            autoResize();
        });
    }

    function autoResize() {
        if (!textareaEl) return;
        const max = window.innerHeight * 0.4;
        textareaEl.style.height = 'auto';
        const clamped = Math.min(textareaEl.scrollHeight, max);
        textareaEl.style.height = clamped + 'px';
        textareaEl.style.overflowY = textareaEl.scrollHeight > max ? 'auto' : 'hidden';
    }

    async function pickFile() {
        const selected = await open({ multiple: false, directory: false });
        if (selected) {
            pendingAttachment = { path: selected as string, temp: false };
        }
    }

    function clearAttachment() {
        pendingAttachment = null;
    }

    function fileName(path: string): string {
        return path.split('/').pop() ?? path;
    }

    async function handlePaste(event: ClipboardEvent) {
        const path = await invoke<string | null>('paste_clipboard_image');
        if (path) {
            pendingAttachment = { path, temp: true };
        }
    }

    function handleClickOutside(event: MouseEvent) {
        if (showEmojiPicker && pickerAnchorEl && !pickerAnchorEl.contains(event.target as Node)) {
            showEmojiPicker = false;
        }
    }

    onMount(() => {
        document.addEventListener('click', handleClickOutside, true);
    });
    onDestroy(() => document.removeEventListener('click', handleClickOutside, true));

    const EMOJI_CATEGORIES: { label: string; emojis: string[] }[] = [
        { label: 'Smileys', emojis: [
            '😀','😃','😄','😁','😆','😅','🤣','😂','🙂','😊',
            '😇','🥰','😍','🤩','😘','😋','😛','😜','🤪','😝',
            '🤑','🤗','🤭','🤫','🤔','😐','😑','😶','😏','😒',
            '🙄','😬','😮‍💨','🤥','😌','😔','😪','🤤','😴','😷',
            '🤒','🤕','🤢','🤮','🥵','🥶','🥴','😵','🤯','🤠',
            '🥳','🥸','😎','🤓','🧐','😕','😟','🙁','😮','😯',
            '😲','😳','🥺','🥹','😦','😧','😨','😰','😥','😢',
            '😭','😱','😖','😣','😞','😓','😩','😫','🥱','😤',
            '😡','😠','🤬','😈','👿','💀','☠️','💩','🤡','👹',
        ]},
        { label: 'Gestures', emojis: [
            '👋','🤚','🖐️','✋','🖖','👌','🤌','🤏','✌️','🤞',
            '🤟','🤘','🤙','👈','👉','👆','🖕','👇','☝️','👍',
            '👎','✊','👊','🤛','🤜','👏','🙌','👐','🤲','🤝',
            '🙏','💪','🦾','🫶','🫡','🫰','🫳','🫴',
        ]},
        { label: 'Hearts', emojis: [
            '❤️','🧡','💛','💚','💙','💜','🖤','🤍','🤎','💔',
            '❤️‍🔥','❤️‍🩹','💕','💞','💓','💗','💖','💘','💝','💟',
            '♥️','🩷','🩵','🩶',
        ]},
        { label: 'Animals', emojis: [
            '🐶','🐱','🐭','🐹','🐰','🦊','🐻','🐼','🐻‍❄️','🐨',
            '🐯','🦁','🐮','🐷','🐸','🐵','🙈','🙉','🙊','🐔',
            '🐧','🐦','🐤','🦆','🦅','🦉','🦇','🐺','🐗','🐴',
            '🦄','🐝','🪱','🐛','🦋','🐌','🐞','🐜','🪰','🐢',
            '🐍','🦎','🐙','🦑','🦐','🦀','🐡','🐠','🐟','🐬',
            '🐳','🐋','🦈','🐊',
        ]},
        { label: 'Food', emojis: [
            '🍎','🍐','🍊','🍋','🍌','🍉','🍇','🍓','🫐','🍈',
            '🍒','🍑','🥭','🍍','🥥','🥝','🍅','🥑','🍔','🍟',
            '🍕','🌭','🥪','🌮','🌯','🥗','🍿','🧂','🍩','🍪',
            '🎂','🍰','🧁','🍫','🍬','🍭','☕','🍵','🧃','🥤',
            '🍺','🍻','🥂','🍷',
        ]},
        { label: 'Objects', emojis: [
            '⌨️','🖥️','💻','📱','☎️','📷','🎥','🔦','💡','📖',
            '💰','💎','🔧','🔨','⚙️','🔗','📎','✂️','📝','📌',
            '🔒','🔓','🔑','🗑️','📦','📫','🏷️','🔔','🎵','🎶',
            '🎤','🎧','🎮','🎲','🎯','🏆','🥇','🥈','🥉','⚽',
            '🏀','🏈','⚾','🎾',
        ]},
        { label: 'Symbols', emojis: [
            '✅','❌','❓','❗','‼️','⁉️','💯','🔥','⭐','✨',
            '💫','💥','💢','💤','🕳️','💬','👁️‍🗨️','🗨️','💭','🚩',
            '🏳️','🏴','✔️','➕','➖','➗','✖️','♾️','🔴','🟠',
            '🟡','🟢','🔵','🟣','⚫','⚪','🟤',
        ]},
    ];

    let activeCategory = EMOJI_CATEGORIES[0].label;

    function insertEmoji(emoji: string) {
        const start = textareaEl.selectionStart;
        const end = textareaEl.selectionEnd;
        messageText = messageText.slice(0, start) + emoji + messageText.slice(end);
        showEmojiPicker = false;
        // Restore focus and cursor position after the inserted emoji
        requestAnimationFrame(() => {
            textareaEl.focus();
            const pos = start + emoji.length;
            textareaEl.selectionStart = pos;
            textareaEl.selectionEnd = pos;
            autoResize();
        });
    }

    async function submit() {
        const trimmed = messageText.trim();
        if (!trimmed && !pendingAttachment) return;

        const roomId = get(activeChannelId);
        if (!roomId) return;
        const reply = get(replyingTo);
        const body = reply
            ? `> ${reply.sender}: ${reply.body}\n\n${trimmed}`
            : trimmed;

        const attachment = pendingAttachment;
        const mentions = new Map(mentionMap);

        // Save state for recovery on failure
        const savedText = messageText;
        const savedAttachment = pendingAttachment;
        const savedMentions = new Map(mentionMap);

        // Optimistic clear
        messageText = '';
        pendingAttachment = null;
        mentionMap.clear();
        clearReply();
        requestAnimationFrame(autoResize);

        try {
            if (body) {
                const rawHtml = composeHtml(body);
                const withMentions = insertMentionLinks(rawHtml, mentions);
                const needsHtml = mentions.size > 0 || withMentions !== `<p>${body}</p>\n`;
                await sendMessage(roomId, body, needsHtml ? withMentions : null, null);
            }
            if (attachment) {
                await sendMessage(roomId, '', null, attachment.path);
                if (attachment.temp) {
                    try { await remove(attachment.path); } catch {}
                }
            }
        } catch {
            // Restore draft so the user doesn't lose their message
            messageText = savedText;
            pendingAttachment = savedAttachment;
            for (const [k, v] of savedMentions) mentionMap.set(k, v);
            requestAnimationFrame(autoResize);
        }
    }

    function handleKeydown(event: KeyboardEvent) {
        if (event.key === 'Tab' && !event.shiftKey) {
            event.preventDefault();
            handleTabCompletion();
            return;
        }

        // Any non-Tab key resets cycling state
        if (tabIndex >= 0) {
            tabMatches = [];
            tabIndex = -1;
        }

        if (event.key === 'Enter' && !event.shiftKey) {
            if (composeLock) return;
            event.preventDefault();
            submit();
        }
    }

    function truncate(text: string, max = 80): string {
        return text.length > max ? text.slice(0, max) + '…' : text;
    }
</script>

<div class="input-wrapper" class:compose-locked={composeLock}>
    {#if $replyingTo}
        <div class="reply-preview">
            <div class="reply-info">
                <svg width="12" height="12" viewBox="0 0 24 24" class="reply-icon">
                    <path fill="currentColor" d="M10 9V5L3 12L10 19V14.9C15 14.9 18.5 16.5 21 20C20 15 17 10 10 9Z"/>
                </svg>
                <span class="reply-sender">{$replyingTo.sender.split(':')[0]}</span>
                <span class="reply-body">{truncate($replyingTo.body)}</span>
            </div>
            <button class="cancel-reply" aria-label="Cancel reply" on:click={clearReply}>
                <svg width="14" height="14" viewBox="0 0 24 24">
                    <path fill="currentColor" d="M18.4 4L12 10.4L5.6 4L4 5.6L10.4 12L4 18.4L5.6 20L12 13.6L18.4 20L20 18.4L13.6 12L20 5.6L18.4 4Z"/>
                </svg>
            </button>
        </div>
    {/if}

    {#if pendingAttachment}
        <div class="attachment-preview">
            <div class="attachment-info">
                <svg width="14" height="14" viewBox="0 0 24 24" class="attachment-icon">
                    <path fill="currentColor" d="M14 2H6C4.9 2 4 2.9 4 4V20C4 21.1 4.9 22 6 22H18C19.1 22 20 21.1 20 20V8L14 2ZM18 20H6V4H13V9H18V20Z"/>
                </svg>
                <span class="attachment-name">{fileName(pendingAttachment.path)}</span>
            </div>
            <button class="cancel-attachment" aria-label="Remove attachment" on:click={clearAttachment}>
                <svg width="14" height="14" viewBox="0 0 24 24">
                    <path fill="currentColor" d="M18.4 4L12 10.4L5.6 4L4 5.6L10.4 12L4 18.4L5.6 20L12 13.6L18.4 20L20 18.4L13.6 12L20 5.6L18.4 4Z"/>
                </svg>
            </button>
        </div>
    {/if}

    <div class="input-container">
        <button class="icon-button attach-button" aria-label="Attach file" on:click={pickFile}>
            <svg width="24" height="24" viewBox="0 0 24 24">
                <path fill="currentColor" fill-rule="evenodd" clip-rule="evenodd" d="M12 2C6.48 2 2 6.48 2 12C2 17.52 6.48 22 12 22C17.52 22 22 17.52 22 12C22 6.48 17.52 2 12 2ZM13 11H16V13H13V16H11V13H8V11H11V8H13V11Z"></path>
            </svg>
        </button>

        <textarea
            class="message-box"
            placeholder="Message #{$activeChannel?.display_name ?? 'general'}"
            bind:value={messageText}
            bind:this={textareaEl}
            on:keydown={handleKeydown}
            on:paste={handlePaste}
            on:input={autoResize}
            rows="1"
        ></textarea>

        <div class="action-buttons">
            <button
                class="icon-button lock-button"
                class:active={composeLock}
                aria-label={composeLock ? 'Unlock send' : 'Lock send (compose mode)'}
                on:click={() => composeLock = !composeLock}
            >
                {#if composeLock}
                    <svg width="20" height="20" viewBox="0 0 24 24">
                        <path fill="currentColor" d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zM9 8V6c0-1.66 1.34-3 3-3s3 1.34 3 3v2H9z"/>
                    </svg>
                {:else}
                    <svg width="20" height="20" viewBox="0 0 24 24">
                        <path fill="currentColor" d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6h2c0-1.66 1.34-3 3-3s3 1.34 3 3v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm0 12H6V10h12v10zm-6-3c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2z"/>
                    </svg>
                {/if}
            </button>

            {#if composeLock}
                <button class="icon-button send-button" aria-label="Send message" on:click={submit}>
                    <svg width="20" height="20" viewBox="0 0 24 24">
                        <path fill="currentColor" d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z"/>
                    </svg>
                </button>
            {/if}

            <div class="emoji-picker-anchor" bind:this={pickerAnchorEl}>
                <button class="icon-button" aria-label="Emoji" on:click={() => showEmojiPicker = !showEmojiPicker}>
                    <svg width="24" height="24" viewBox="0 0 24 24">
                        <path fill="currentColor" fill-rule="evenodd" clip-rule="evenodd" d="M12 2C6.486 2 2 6.486 2 12C2 17.515 6.486 22 12 22C17.514 22 22 17.515 22 12C22 6.486 17.514 2 12 2ZM8.5 8C9.328 8 10 8.671 10 9.5C10 10.329 9.328 11 8.5 11C7.672 11 7 10.329 7 9.5C7 8.671 7.672 8 8.5 8ZM12 17.5C9.666 17.5 7.655 15.967 6.88 13.84L8.766 13.19C9.255 14.536 10.536 15.5 12 15.5C13.464 15.5 14.745 14.536 15.234 13.19L17.12 13.84C16.345 15.967 14.334 17.5 12 17.5ZM15.5 11C14.672 11 14 10.329 14 9.5C14 8.671 14.672 8 15.5 8C16.328 8 17 8.671 17 9.5C17 10.329 16.328 11 15.5 11Z"></path>
                    </svg>
                </button>

                {#if showEmojiPicker}
                    <div class="emoji-picker">
                        <div class="emoji-tabs">
                            {#each EMOJI_CATEGORIES as cat}
                                <button
                                    class="emoji-tab {activeCategory === cat.label ? 'active' : ''}"
                                    on:click={() => activeCategory = cat.label}
                                >{cat.emojis[0]}</button>
                            {/each}
                        </div>
                        <div class="emoji-grid">
                            {#each EMOJI_CATEGORIES as cat}
                                {#if activeCategory === cat.label}
                                    {#each cat.emojis as emoji}
                                        <button
                                            class="emoji-cell"
                                            on:click={() => insertEmoji(emoji)}
                                            aria-label={emoji}
                                        >{emoji}</button>
                                    {/each}
                                {/if}
                            {/each}
                        </div>
                    </div>
                {/if}
            </div>
        </div>
    </div>
</div>

<style>
    .input-wrapper {
        width: 100%;
        background-color: transparent;
        border-radius: 10px;
        border: 2px solid transparent;
        transition: border-color 0.15s ease;
    }

    .input-wrapper.compose-locked {
        border-color: var(--accent);
    }

    .reply-preview {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 6px 16px 4px 16px;
        border-bottom: 1px solid var(--bg-hover);
    }

    .reply-info {
        display: flex;
        align-items: center;
        gap: 6px;
        min-width: 0;
        color: #b9bbbe;
        font-size: 13px;
    }

    .reply-icon { flex-shrink: 0; color: var(--accent); }

    .reply-sender { font-weight: 600; color: #dcddde; white-space: nowrap; }

    .reply-body {
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        color: #8e9297;
    }

    .cancel-reply {
        flex-shrink: 0;
        background: none;
        border: none;
        color: #72767d;
        cursor: pointer;
        padding: 2px;
        display: flex;
        align-items: center;
        border-radius: 3px;
        transition: color 0.1s;
    }

    .cancel-reply:hover { color: #dcddde; }

    .attachment-preview {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 6px 16px 4px 16px;
        border-bottom: 1px solid var(--bg-hover);
    }

    .attachment-info {
        display: flex;
        align-items: center;
        gap: 6px;
        min-width: 0;
        color: #b9bbbe;
        font-size: 13px;
    }

    .attachment-icon { flex-shrink: 0; color: var(--accent); }

    .attachment-name {
        font-weight: 500;
        color: #dcddde;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .cancel-attachment {
        flex-shrink: 0;
        background: none;
        border: none;
        color: #72767d;
        cursor: pointer;
        padding: 2px;
        display: flex;
        align-items: center;
        border-radius: 3px;
        transition: color 0.1s;
    }

    .cancel-attachment:hover { color: #dcddde; }

    .input-container {
        display: flex;
        align-items: center;
        border-radius: 8px;
        padding: 4px 16px;
        min-height: 44px;
    }

    .icon-button {
        background: none;
        border: none;
        padding: 0;
        margin: 0;
        cursor: pointer;
        color: #b9bbbe;
        display: flex;
        align-items: center;
        justify-content: center;
        transition: color 0.1s ease;
    }

    .icon-button:hover { color: #dcddde; }

    .lock-button.active { color: var(--accent); }
    .lock-button.active:hover { color: var(--accent-hover); }

    .send-button { color: var(--accent); }
    .send-button:hover { color: var(--accent-hover); }

    .attach-button { margin-right: 16px; }

    .action-buttons { display: flex; gap: 12px; margin-left: 16px; }

    .message-box {
        flex-grow: 1;
        box-sizing: border-box;
        background: transparent;
        border: none;
        color: #dcddde;
        font-family: 'Inter', sans-serif;
        font-size: 16px;
        line-height: 22px;
        padding: 11px 0;
        resize: none;
        outline: none;
        overflow-y: hidden;
        -webkit-user-select: text;
        user-select: text;
    }

    .emoji-picker-anchor { position: relative; }

    .emoji-picker {
        position: absolute;
        bottom: 40px;
        right: 0;
        width: 352px;
        height: 360px;
        background-color: #2f3136;
        border: 1px solid #202225;
        border-radius: 8px;
        display: flex;
        flex-direction: column;
        z-index: 20;
        box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    }

    .emoji-tabs {
        display: flex;
        border-bottom: 1px solid #202225;
        padding: 4px 4px 0;
    }

    .emoji-tab {
        flex: 1;
        background: none;
        border: none;
        border-bottom: 2px solid transparent;
        padding: 6px 0;
        font-size: 18px;
        cursor: pointer;
        border-radius: 4px 4px 0 0;
        transition: background-color 0.1s;
    }

    .emoji-tab:hover { background-color: var(--bg-hover); }
    .emoji-tab.active { border-bottom-color: var(--accent); }

    .emoji-grid {
        display: grid;
        grid-template-columns: repeat(8, 1fr);
        gap: 2px;
        padding: 8px;
        overflow-y: auto;
        flex: 1;
    }

    .emoji-grid::-webkit-scrollbar { width: 6px; }
    .emoji-grid::-webkit-scrollbar-track { background: transparent; }
    .emoji-grid::-webkit-scrollbar-thumb { background-color: #202225; border-radius: 3px; }

    .emoji-cell {
        width: 36px;
        height: 36px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: none;
        border: none;
        border-radius: 4px;
        font-size: 22px;
        cursor: pointer;
        transition: background-color 0.1s;
    }

    .emoji-cell:hover { background-color: var(--bg-hover); }

    .message-box::placeholder { color: #72767d; }

    .message-box::-webkit-scrollbar { width: 4px; }
    .message-box::-webkit-scrollbar-track { background: transparent; }
    .message-box::-webkit-scrollbar-thumb { background-color: #202225; border-radius: 4px; }
</style>
