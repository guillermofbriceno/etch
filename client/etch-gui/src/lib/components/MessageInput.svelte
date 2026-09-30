<script lang="ts">
    import { onMount, onDestroy, tick } from 'svelte';
    import { get } from 'svelte/store';
    import { open } from '@tauri-apps/plugin-dialog';
    import { stat } from '@tauri-apps/plugin-fs';
    import { invoke } from '@tauri-apps/api/core';
    import { getCurrentWebview, type DragDropEvent } from '@tauri-apps/api/webview';
    import type { Event as TauriEvent } from '@tauri-apps/api/event';
    import { sendMessage, editMessage, activeChannelId, activeChannel, activeWindow, replyingTo, clearReply, editingMessage, clearEditing, activeOverlay, uploadLimits, showToast } from '$lib/stores';
    import { composeHtml, insertMentionLinks } from '$lib/markdown';
    import {
        COMPRESS_THRESHOLD_BYTES,
        checkAttachment,
        compressionFailedReason,
        discardTempFile,
        fileName,
        isCompressible,
        isTempPath,
        limitFor,
        overLimitReason,
        probeMediaInfo,
        writeTempAttachment,
    } from '$lib/attachments';
    import { formatMB, formatSize } from '$lib/media';
    import Icon from './Icon.svelte';
    import { customScrollbar } from '$lib/scrollbar';

    type Attachment = { path: string; size: number };

    let messageText = '';
    let showEmojiPicker = false;
    let textareaEl: HTMLTextAreaElement;
    let pickerAnchorEl: HTMLDivElement;
    let pendingAttachment: Attachment | null = null;
    let compressAttachment = true;
    let processingPaste = false;
    let composeLock = false;
    let dragActive = false;

    $: attachmentLimit = pendingAttachment ? limitFor(pendingAttachment.path, $uploadLimits) : null;
    $: verdict = pendingAttachment ? checkAttachment(pendingAttachment.path, pendingAttachment.size, $uploadLimits) : null;
    $: mustCompress = verdict !== null && verdict.accept && verdict.mustCompress;
    $: if (mustCompress) compressAttachment = true;
    $: showCompress = pendingAttachment !== null
        && isCompressible(pendingAttachment.path)
        && (mustCompress || pendingAttachment.size > COMPRESS_THRESHOLD_BYTES);
    $: inputActive = $activeChannelId !== null && $activeOverlay === 'none' && !$editingMessage;

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

    // Populate input when entering edit mode
    let prevEditingId: string | null = null;
    $: {
        const id = $editingMessage?.id ?? null;
        if (id !== prevEditingId) {
            prevEditingId = id;
            if ($editingMessage) {
                messageText = $editingMessage.body;
                requestAnimationFrame(() => { autoResize(); textareaEl?.focus(); });
            }
        }
    }

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

    function setPending(next: Attachment | null) {
        const previous = pendingAttachment;
        pendingAttachment = next;
        compressAttachment = true;
        if (previous && previous.path !== next?.path) discardTempFile(previous.path);
    }

    function rejectedOverLimit(name: string, size: number): boolean {
        const verdict = checkAttachment(name, size, get(uploadLimits));
        if (verdict.accept) return false;
        showToast(`Couldn't attach ${name}: ${verdict.reason}`);
        return true;
    }

    // A slower attach that finishes after a newer one must not replace it.
    let attachSeq = 0;
    async function attach(path: string, knownSize?: number) {
        const seq = ++attachSeq;
        let size = knownSize;
        if (size === undefined) {
            try {
                size = (await stat(path)).size;
            } catch {
                if (seq === attachSeq) showToast(`Couldn't attach ${fileName(path)}: the file could not be read`);
                return;
            }
        }
        if (seq !== attachSeq || rejectedOverLimit(fileName(path), size)) {
            discardTempFile(path);
            return;
        }
        setPending({ path, size });
    }

    async function attachPastedFile(file: File) {
        const name = file.name || 'attachment';
        if (rejectedOverLimit(name, file.size)) return;
        let path: string;
        try {
            path = await writeTempAttachment(name, new Uint8Array(await file.arrayBuffer()));
        } catch {
            showToast(`Couldn't attach ${name}: the file could not be read`);
            return;
        }
        await attach(path, file.size);
    }

    async function pickFile() {
        const selected = await open({ multiple: false, directory: false });
        if (selected) await attach(selected);
    }

    function clearAttachment() {
        setPending(null);
    }

    let pasteInFlight = false;
    async function handlePaste(event: ClipboardEvent) {
        if (pasteInFlight) return;
        // The clipboard's file list is emptied once the event returns.
        const files = Array.from(event.clipboardData?.files ?? []);
        pasteInFlight = true;
        const spinnerDelay = setTimeout(() => { processingPaste = true; }, 100);
        try {
            // The shell answers null when there is no image, and errors only when the clipboard or its image could not be read.
            let failure: unknown = null;
            const bitmap = await invoke<[string, number] | null>('paste_clipboard_image').catch((e) => {
                failure = e;
                return null;
            });
            if (bitmap) {
                const [path, size] = bitmap;
                await attach(path, size);
            } else if (files.length > 0) {
                await attachPastedFile(files[0]);
            } else if (failure !== null) {
                showToast(`Couldn't paste the image: ${failure}`);
            }
        } finally {
            clearTimeout(spinnerDelay);
            processingPaste = false;
            pasteInFlight = false;
        }
    }

    function handleDragDrop(event: TauriEvent<DragDropEvent>) {
        const drag = event.payload;
        if (drag.type === 'enter') {
            dragActive = inputActive && drag.paths.length > 0;
        } else if (drag.type === 'leave') {
            dragActive = false;
        } else if (drag.type === 'drop') {
            dragActive = false;
            if (inputActive && drag.paths.length > 0) attach(drag.paths[0]);
        }
    }

    function handleClickOutside(event: MouseEvent) {
        if (showEmojiPicker && pickerAnchorEl && !pickerAnchorEl.contains(event.target as Node)) {
            showEmojiPicker = false;
        }
    }

    let destroyed = false;
    let stopDragDrop: (() => void) | null = null;

    onMount(() => {
        document.addEventListener('click', handleClickOutside, true);
        getCurrentWebview().onDragDropEvent(handleDragDrop).then((unlisten) => {
            if (destroyed) unlisten();
            else stopDragDrop = unlisten;
        });
    });
    onDestroy(() => {
        destroyed = true;
        document.removeEventListener('click', handleClickOutside, true);
        stopDragDrop?.();
    });

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

    async function sendAttachment(roomId: string, attachment: Attachment, wantCompress: boolean, onPrepared: (prepared: Attachment) => void) {
        const name = fileName(attachment.path);
        const limit = limitFor(attachment.path, get(uploadLimits));
        const shrinkTo = limit !== null && attachment.size > limit ? limit : null;
        let path = attachment.path;
        let size: number | null = attachment.size;

        const abandon = (reason: string) => {
            showToast(`Couldn't send ${name}: ${reason}`);
            if (path !== attachment.path) discardTempFile(path);
            discardTempFile(attachment.path);
        };

        if (isCompressible(path) && (shrinkTo !== null || (wantCompress && attachment.size > COMPRESS_THRESHOLD_BYTES))) {
            try {
                path = await invoke<string>('compress_image', { path: attachment.path });
            } catch {
                if (shrinkTo !== null) return abandon(compressionFailedReason(shrinkTo));
            }
            if (path !== attachment.path) {
                // Null when the stat fails, which leaves the size check to core.
                size = await stat(path).then(meta => meta.size, () => null);
            } else if (shrinkTo !== null) {
                return abandon(compressionFailedReason(shrinkTo));
            }
        }
        if (limit !== null && size !== null && size > limit) {
            return abandon(overLimitReason(size, limit, path !== attachment.path));
        }

        const prepared = { path, size: size ?? attachment.size };
        onPrepared(prepared);
        const mediaInfo = await probeMediaInfo(prepared.path, prepared.size);
        await sendMessage(roomId, '', null, prepared.path, mediaInfo);
    }

    // Anything typed or attached while the send was in flight wins over the restored draft.
    function restoreText(text: string, mentions: Map<string, string>) {
        if (!text.trim() || messageText.trim() || get(editingMessage)) return;
        messageText = text;
        for (const [k, v] of mentions) mentionMap.set(k, v);
        requestAnimationFrame(autoResize);
    }

    function restoreAttachment(original: Attachment, prepared: Attachment | null) {
        let restored = original;
        if (prepared && prepared.path !== original.path) {
            // compress_image deletes a temp input it replaced, while a picked source still exists.
            if (isTempPath(original.path)) restored = prepared;
            else discardTempFile(prepared.path);
        }
        if (pendingAttachment === null) pendingAttachment = restored;
        else discardTempFile(restored.path);
    }

    async function submit() {
        const trimmed = messageText.trim();
        if (!trimmed && !pendingAttachment) return;

        const roomId = get(activeChannelId);
        if (!roomId) return;

        // Edit mode: fire-and-forget update. The timeline diff stream will
        // reflect success (optimistic local echo) or revert on server rejection.
        const editing = get(editingMessage);
        if (editing) {
            const rawHtml = composeHtml(trimmed);
            const withMentions = insertMentionLinks(rawHtml, new Map(mentionMap));
            const needsHtml = mentionMap.size > 0 || withMentions !== `<p>${trimmed}</p>\n`;

            messageText = '';
            clearEditing();
            mentionMap.clear();
            requestAnimationFrame(autoResize);

            editMessage(roomId, editing.id, trimmed, needsHtml ? withMentions : null);
            return;
        }

        const reply = get(replyingTo);
        const body = reply
            ? reply.body.split('\n').map((l: string, i: number) =>
                  i === 0 ? `> ${reply.sender}: ${l}` : `> ${l}`
              ).join('\n') + `\n\n${trimmed}`
            : trimmed;

        const attachment = pendingAttachment;
        const shouldCompress = compressAttachment;
        const mentions = new Map(mentionMap);

        const savedText = messageText;
        const savedMentions = new Map(mentionMap);

        // Optimistic clear
        messageText = '';
        pendingAttachment = null;
        compressAttachment = true;
        mentionMap.clear();
        clearReply();
        requestAnimationFrame(autoResize);

        let textSent = false;
        let failing = 'your message';
        let prepared = null as Attachment | null;
        try {
            if (body) {
                const rawHtml = composeHtml(body);
                const withMentions = insertMentionLinks(rawHtml, mentions);
                const needsHtml = mentions.size > 0 || withMentions !== `<p>${body}</p>\n`;
                await sendMessage(roomId, body, needsHtml ? withMentions : null, null);
                textSent = true;
            }
            if (attachment) {
                failing = fileName(attachment.path);
                await sendAttachment(roomId, attachment, shouldCompress, (next) => { prepared = next; });
            }
        } catch (e) {
            showToast(`Couldn't send ${failing}: ${e}`);
            if (!textSent) restoreText(savedText, savedMentions);
            if (attachment) restoreAttachment(attachment, prepared);
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

        if (event.key === 'Escape' && get(editingMessage)) {
            event.preventDefault();
            messageText = '';
            clearEditing();
            requestAnimationFrame(autoResize);
            return;
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

<div class="input-wrapper" class:compose-locked={composeLock} class:drop-target={dragActive && inputActive}>
    {#if $editingMessage}
        <div class="reply-preview editing-preview">
            <div class="reply-info">
                <Icon name="edit" size={12} class="reply-icon" />
                <span class="reply-sender">Editing</span>
                <span class="reply-body">{truncate($editingMessage.body)}</span>
            </div>
            <button class="cancel-reply" aria-label="Cancel edit" on:click={() => { messageText = ''; clearEditing(); requestAnimationFrame(autoResize); }}>
                <Icon name="close" size={14} />
            </button>
        </div>
    {:else if $replyingTo}
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
                <span class="attachment-size">{formatSize(pendingAttachment.size)}</span>
            </div>
            <div class="attachment-actions">
                {#if showCompress}
                    {#if mustCompress && attachmentLimit !== null}
                        <span class="compress-note">Must be compressed to fit the {formatMB(attachmentLimit)} limit</span>
                    {/if}
                    <label class="compress-option" class:forced={mustCompress}>
                        <input type="checkbox" bind:checked={compressAttachment} disabled={mustCompress} />
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
        <div class="mention-popup" style="--scrollbar-thumb: var(--border-input)" use:customScrollbar={{ width: 6, minThumbHeight: 20 }}>
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
                        <div class="emoji-grid" use:customScrollbar={{ width: 6, minThumbHeight: 20 }}>
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

    .input-wrapper.drop-target {
        border-color: var(--accent);
        border-style: dashed;
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
        color: var(--text-secondary);
        font-size: 13px;
    }

    .reply-info :global(.reply-icon) { flex-shrink: 0; color: var(--accent); }

    .reply-sender { font-weight: 600; color: var(--text-primary); white-space: nowrap; }

    .reply-body {
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        color: var(--text-tertiary);
    }

    .cancel-reply {
        flex-shrink: 0;
        background: none;
        border: none;
        color: var(--text-muted);
        cursor: pointer;
        padding: 2px;
        display: flex;
        align-items: center;
        border-radius: 3px;
        transition: color 0.1s;
    }

    .cancel-reply:hover { color: var(--text-primary); }

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
        color: var(--text-secondary);
        font-size: 13px;
    }

    .attachment-info :global(.attachment-icon) { flex-shrink: 0; color: var(--accent); }

    .attachment-name {
        font-weight: 500;
        color: var(--text-primary);
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
        color: var(--text-secondary);
        font-size: 13px;
        user-select: none;
    }

    .compress-option input[type="checkbox"] {
        accent-color: var(--primary);
        width: 14px;
        height: 14px;
        margin: 0;
        cursor: pointer;
    }

    .compress-option.forced,
    .compress-option.forced input[type="checkbox"] { cursor: default; }

    .compress-note {
        color: var(--text-muted);
        font-size: 12px;
        white-space: nowrap;
    }

    .attachment-size {
        flex-shrink: 0;
        color: var(--text-muted);
        font-size: 12px;
    }

    .spinner {
        width: 14px;
        height: 14px;
        border: 2px solid rgba(255, 255, 255, 0.1);
        border-top-color: var(--text-secondary);
        border-radius: 50%;
        animation: spin 0.8s linear infinite;
    }

    @keyframes spin { to { transform: rotate(360deg); } }

    .cancel-attachment {
        flex-shrink: 0;
        background: none;
        border: none;
        color: var(--text-muted);
        cursor: pointer;
        padding: 2px;
        display: flex;
        align-items: center;
        border-radius: 3px;
        transition: color 0.1s;
    }

    .cancel-attachment:hover { color: var(--text-primary); }

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
        color: var(--text-secondary);
        display: flex;
        align-items: center;
        justify-content: center;
        transition: color 0.1s ease;
    }

    .icon-button:hover { color: var(--text-primary); }

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
        color: var(--text-primary);
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
        border: 1px solid var(--border-subtle);
        border-radius: 8px;
        display: flex;
        flex-direction: column;
        z-index: 20;
        box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    }

    .emoji-tabs {
        display: flex;
        border-bottom: 1px solid var(--border-subtle);
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

    .message-box::placeholder { color: var(--text-muted); }


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


    .mention-option {
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        padding: 8px 12px;
        background: none;
        border: none;
        border-radius: 4px;
        color: var(--text-primary);
        font-size: var(--font-size-base);
        cursor: pointer;
        text-align: left;
    }

    .mention-option:hover,
    .mention-option.selected {
        background-color: var(--bg-hover);
    }

    .mention-name { font-weight: 500; }
    .mention-id { color: var(--text-muted); font-size: 12px; }
</style>
