<script lang="ts">
    import { onMount, onDestroy, tick } from 'svelte';
    import { get } from 'svelte/store';
    import { open } from '@tauri-apps/plugin-dialog';
    import { stat } from '@tauri-apps/plugin-fs';
    import { invoke } from '@tauri-apps/api/core';
    import { sendMessage, activeChannelId, activeChannel, activeWindow, replyingTo, clearReply } from '$lib/stores';
    import { composeHtml, insertMentionLinks } from '$lib/markdown';
    import Icon from './Icon.svelte';

    let messageText = '';
    let showEmojiPicker = false;
    let textareaEl: HTMLTextAreaElement;
    let pickerAnchorEl: HTMLDivElement;
    let pendingAttachment: { path: string; temp: boolean; size: number } | null = null;
    let compressAttachment = true;
    let processingPaste = false;
    let composeLock = false;

    const imageExtensions = ['png', 'jpg', 'jpeg', 'gif', 'bmp', 'webp', 'tiff', 'tif'];
    function isImage(path: string): boolean {
        const ext = path.split('.').pop()?.toLowerCase() ?? '';
        return imageExtensions.includes(ext);
    }

    // Tab-completion state
    let tabPrefix = '';
    let tabMatches: { displayName: string; matrixId: string }[] = [];
    let tabIndex = -1;
    let tabStart = -1;
    let tabEnd = -1;
    // Tracks completed mentions in the current message: displayName -> matrixId
    const mentionMap = new Map<string, string>();

    // Mention popup state
    let showMentionPopup = false;
    let mentionQuery = '';
    let mentionSelectedIndex = 0;

    // Clear mention and tab-completion state when switching channels
    $: $activeChannelId, (() => {
        mentionMap.clear();
        tabMatches = [];
        tabIndex = -1;
        showMentionPopup = false;
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

    $: mentionMatches = showMentionPopup
        ? roomUsers.filter(u =>
            u.displayName.toLowerCase().startsWith(mentionQuery.toLowerCase()) ||
            u.matrixId.slice(1).split(':')[0].toLowerCase().startsWith(mentionQuery.toLowerCase())
          ).slice(0, 8)
        : [];

    function checkMentionTrigger() {
        if (!textareaEl) return;
        const cursor = textareaEl.selectionStart;
        const before = messageText.slice(0, cursor);
        const match = before.match(/@(\S*)$/);
        if (match) {
            mentionQuery = match[1];
            mentionSelectedIndex = 0;
            showMentionPopup = true;
        } else {
            showMentionPopup = false;
        }
    }

    async function selectMention(user: { displayName: string; matrixId: string }) {
        const cursor = textareaEl.selectionStart;
        const before = messageText.slice(0, cursor);
        const match = before.match(/@(\S*)$/);
        if (!match) return;

        const start = cursor - match[0].length;
        const replacement = `@${user.displayName} `;
        messageText = messageText.slice(0, start) + replacement + messageText.slice(cursor);
        mentionMap.set(user.displayName, user.matrixId);
        showMentionPopup = false;

        await tick();
        if (!textareaEl) return;
        const pos = start + replacement.length;
        textareaEl.selectionStart = pos;
        textareaEl.selectionEnd = pos;
        textareaEl.focus();
        autoResize();
    }

    async function handleTabCompletion() {
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

        await tick();
        if (!textareaEl) return;
        textareaEl.selectionStart = tabEnd;
        textareaEl.selectionEnd = tabEnd;
        autoResize();
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
            const meta = await stat(selected as string);
            pendingAttachment = { path: selected as string, temp: false, size: meta.size };
        }
    }

    function clearAttachment() {
        pendingAttachment = null;
    }

    function fileName(path: string): string {
        return path.split('/').pop() ?? path;
    }

    let pasteInFlight = false;
    async function handlePaste(event: ClipboardEvent) {
        if (pasteInFlight) return;
        pasteInFlight = true;
        const spinnerDelay = setTimeout(() => { processingPaste = true; }, 100);
        try {
            const result = await invoke<[string, number] | null>('paste_clipboard_image');
            if (result) {
                const [path, size] = result;
                pendingAttachment = { path, temp: true, size };
            }
        } finally {
            clearTimeout(spinnerDelay);
            processingPaste = false;
            pasteInFlight = false;
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

    async function insertEmoji(emoji: string) {
        const start = textareaEl.selectionStart;
        const end = textareaEl.selectionEnd;
        messageText = messageText.slice(0, start) + emoji + messageText.slice(end);
        showEmojiPicker = false;
        // Restore focus and cursor position after the inserted emoji
        await tick();
        if (!textareaEl) return;
        textareaEl.focus();
        const pos = start + emoji.length;
        textareaEl.selectionStart = pos;
        textareaEl.selectionEnd = pos;
        autoResize();
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
        const shouldCompress = compressAttachment;
        const mentions = new Map(mentionMap);

        // Save state for recovery on failure
        const savedText = messageText;
        const savedAttachment = pendingAttachment;
        const savedMentions = new Map(mentionMap);

        // Optimistic clear
        messageText = '';
        pendingAttachment = null;
        compressAttachment = true;
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
                let attachPath = attachment.path;
                if (shouldCompress && isImage(attachment.path) && attachment.size > 256_000) {
                    attachPath = await invoke<string>('compress_image', { path: attachment.path });
                }
                await sendMessage(roomId, '', null, attachPath);
            }
        } catch {
            // Restore draft so the user doesn't lose their message
            messageText = savedText;
            pendingAttachment = savedAttachment;
            for (const [k, v] of savedMentions) mentionMap.set(k, v);
            requestAnimationFrame(autoResize);
        }
    }

    async function handleKeydown(event: KeyboardEvent) {
        // Mention popup keyboard navigation
        if (showMentionPopup && mentionMatches.length > 0) {
            if (event.key === 'ArrowDown') {
                event.preventDefault();
                mentionSelectedIndex = (mentionSelectedIndex + 1) % mentionMatches.length;
                return;
            }
            if (event.key === 'ArrowUp') {
                event.preventDefault();
                mentionSelectedIndex = (mentionSelectedIndex - 1 + mentionMatches.length) % mentionMatches.length;
                return;
            }
            if (event.key === 'Tab' || (event.key === 'Enter' && !event.shiftKey)) {
                event.preventDefault();
                await selectMention(mentionMatches[mentionSelectedIndex]);
                return;
            }
            if (event.key === 'Escape') {
                event.preventDefault();
                showMentionPopup = false;
                return;
            }
        }

        if (event.key === 'Tab' && !event.shiftKey) {
            event.preventDefault();
            await handleTabCompletion();
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
                <Icon name="reply" size={12} class="reply-icon" />
                <span class="reply-sender">{$replyingTo.sender.split(':')[0]}</span>
                <span class="reply-body">{truncate($replyingTo.body)}</span>
            </div>
            <button class="cancel-reply" aria-label="Cancel reply" on:click={clearReply}>
                <Icon name="close" size={14} />
            </button>
        </div>
    {/if}

    {#if processingPaste}
        <div class="attachment-preview">
            <div class="attachment-info">
                <div class="spinner"></div>
                <span class="attachment-name">Processing paste...</span>
            </div>
        </div>
    {:else if pendingAttachment}
        <div class="attachment-preview">
            <div class="attachment-info">
                <Icon name="file" size={14} class="attachment-icon" />
                <span class="attachment-name">{fileName(pendingAttachment.path)}</span>
            </div>
            <div class="attachment-actions">
                {#if isImage(pendingAttachment.path) && pendingAttachment.size > 256_000}
                    <label class="compress-option">
                        <input type="checkbox" bind:checked={compressAttachment} />
                        Compress
                    </label>
                {/if}
                <button class="cancel-attachment" aria-label="Remove attachment" on:click={clearAttachment}>
                    <Icon name="close" size={14} />
                </button>
            </div>
        </div>
    {/if}

    {#if showMentionPopup && mentionMatches.length > 0}
        <div class="mention-popup">
            {#each mentionMatches as user, i}
                <button
                    class="mention-option"
                    class:selected={i === mentionSelectedIndex}
                    on:mousedown|preventDefault={() => selectMention(user)}
                    on:mouseenter={() => mentionSelectedIndex = i}
                >
                    <span class="mention-name">{user.displayName}</span>
                    <span class="mention-id">{user.matrixId}</span>
                </button>
            {/each}
        </div>
    {/if}

    <div class="input-container">
        <button class="icon-button attach-button" aria-label="Attach file" on:click={pickFile}>
            <Icon name="plus_circle" />
        </button>

        <textarea
            class="message-box"
            placeholder="Message #{$activeChannel?.display_name ?? 'general'}"
            bind:value={messageText}
            bind:this={textareaEl}
            on:keydown={handleKeydown}
            on:paste={handlePaste}
            on:input={() => { autoResize(); checkMentionTrigger(); }}
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
                    <Icon name="lock" size={20} />
                {:else}
                    <Icon name="lock_open" size={20} />
                {/if}
            </button>

            {#if composeLock}
                <button class="icon-button send-button" aria-label="Send message" on:click={submit}>
                    <Icon name="send" size={20} />
                </button>
            {/if}

            <div class="emoji-picker-anchor" bind:this={pickerAnchorEl}>
                <button class="icon-button" aria-label="Emoji" on:click={() => showEmojiPicker = !showEmojiPicker}>
                    <Icon name="emoji" />
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
        position: relative;
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

    .reply-info :global(.reply-icon) { flex-shrink: 0; color: var(--accent); }

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

    .attachment-info :global(.attachment-icon) { flex-shrink: 0; color: var(--accent); }

    .attachment-name {
        font-weight: 500;
        color: #dcddde;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .attachment-actions {
        display: flex;
        align-items: center;
        gap: 12px;
        flex-shrink: 0;
    }

    .compress-option {
        display: flex;
        align-items: center;
        gap: 6px;
        cursor: pointer;
        color: #b9bbbe;
        font-size: 13px;
        user-select: none;
    }

    .compress-option input[type="checkbox"] {
        accent-color: #5865f2;
        width: 14px;
        height: 14px;
        margin: 0;
        cursor: pointer;
    }

    .spinner {
        width: 14px;
        height: 14px;
        border: 2px solid rgba(255, 255, 255, 0.1);
        border-top-color: #b9bbbe;
        border-radius: 50%;
        animation: spin 0.8s linear infinite;
    }

    @keyframes spin { to { transform: rotate(360deg); } }

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

    .mention-popup {
        position: absolute;
        bottom: 100%;
        left: 16px;
        right: 16px;
        max-height: 240px;
        overflow-y: auto;
        background-color: var(--bg-inset);
        border: 1px solid var(--border-input);
        border-radius: 8px;
        padding: 4px;
        z-index: 20;
        box-shadow: 0 -4px 16px rgba(0, 0, 0, 0.3);
    }

    .mention-popup::-webkit-scrollbar { width: 6px; }
    .mention-popup::-webkit-scrollbar-track { background: transparent; }
    .mention-popup::-webkit-scrollbar-thumb { background-color: var(--border-input); border-radius: 3px; }

    .mention-option {
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        padding: 8px 12px;
        background: none;
        border: none;
        border-radius: 4px;
        color: #dcddde;
        font-size: 14px;
        cursor: pointer;
        text-align: left;
    }

    .mention-option:hover,
    .mention-option.selected {
        background-color: var(--bg-hover);
    }

    .mention-name { font-weight: 500; }
    .mention-id { color: #72767d; font-size: 12px; }
</style>
