<script lang="ts">
    import { onMount, onDestroy, tick } from 'svelte';
    import { get } from 'svelte/store';
    import { open } from '@tauri-apps/plugin-dialog';
    import { remove } from '@tauri-apps/plugin-fs';
    import { invoke } from '@tauri-apps/api/core';
    import { sendMessage, activeChannelId, activeChannel, activeWindow, replyingTo, clearReply } from '$lib/stores';
    import { composeHtml, insertMentionLinks } from '$lib/markdown';
    import Icon from './Icon.svelte';

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

    async function handleKeydown(event: KeyboardEvent) {
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

    {#if pendingAttachment}
        <div class="attachment-preview">
            <div class="attachment-info">
                <Icon name="file" size={14} class="attachment-icon" />
                <span class="attachment-name">{fileName(pendingAttachment.path)}</span>
            </div>
            <button class="cancel-attachment" aria-label="Remove attachment" on:click={clearAttachment}>
                <Icon name="close" size={14} />
            </button>
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
