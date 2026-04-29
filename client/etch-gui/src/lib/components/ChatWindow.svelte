<script lang="ts">
    import { beforeUpdate, afterUpdate, onMount } from 'svelte';
    import { activeWindow, loadOlder, activeChannel, openImage, showRoomIds } from '$lib/stores';
    import type { ChatMessage, TimelineEntry, TimelineEntryKind, StateEventKind } from '$lib/types';
    import MessageGroup from './MessageGroup.svelte';

    let scrollerElement: HTMLDivElement;
    let contentElement: HTMLDivElement;

    // --- Scroll state machine ---
    //
    // Two states: PINNED (stuckAtBottom=true) and BROWSING (stuckAtBottom=false).
    //
    // PINNED: auto-scroll to bottom on new content, content resize, channel switch.
    // BROWSING: never auto-scroll. Show "scroll to bottom" button.
    //
    // Transition PINNED -> BROWSING: user scrolls up past BOTTOM_THRESHOLD.
    // Transition BROWSING -> PINNED: user scrolls back within BOTTOM_THRESHOLD,
    //   or clicks "scroll to bottom" button.
    //
    // Programmatic scrolls (scrollToBottom, scrollBy for prepend) must not
    // trigger state transitions. We track this with `programmaticScroll`.

    const BOTTOM_THRESHOLD = 100; // px — generous to cover subpixel rounding and partial messages
    const TOP_THRESHOLD = 400;   // px — triggers backward pagination when user scrolls near the top
    let stuckAtBottom = true;
    let newMessagesPending = false;
    let programmaticScroll = false;

    // Change detection for afterUpdate
    let prevEntries: TimelineEntry[] = [];
    let prevChannelId: string | undefined = undefined;
    let savedScrollHeight = 0;

    // Suppresses ResizeObserver re-scroll during backward pagination
    let suppressResizeScroll = false;

    // Prevents scroll events from scrollBy() re-triggering pagination after a prepend
    let lastPrependTime = 0;
    const PREPEND_COOLDOWN_MS = 150;

    /** Scroll to the absolute bottom. Defers a second attempt via rAF to handle
     *  cases where layout isn't finalized when afterUpdate runs. */
    function scrollToBottom() {
        if (!scrollerElement) return;
        programmaticScroll = true;
        scrollerElement.scrollTop = scrollerElement.scrollHeight;
        requestAnimationFrame(() => {
            if (!scrollerElement) return;
            programmaticScroll = true;
            scrollerElement.scrollTop = scrollerElement.scrollHeight;
        });
    }

    /** Scroll event handler — only reacts to user-initiated scrolls. */
    function onScroll() {
        if (!scrollerElement) return;
        if (programmaticScroll) {
            programmaticScroll = false;
            return;
        }

        // Bottom detection (PINNED/BROWSING transitions)
        const gap = scrollerElement.scrollHeight - scrollerElement.scrollTop - scrollerElement.clientHeight;
        const wasStuck = stuckAtBottom;
        stuckAtBottom = gap <= BOTTOM_THRESHOLD;
        if (!wasStuck && stuckAtBottom) {
            newMessagesPending = false;
        }

        // Backward pagination: user scrolled near the top
        if (scrollerElement.scrollTop <= TOP_THRESHOLD
            && $activeWindow.hasMore
            && !$activeWindow.loading
            && Date.now() - lastPrependTime > PREPEND_COOLDOWN_MS) {
            loadOlder();
        }
    }

    function jumpToLatest() {
        stuckAtBottom = true;
        newMessagesPending = false;
        scrollToBottom();
    }

    // --- Observers ---

    onMount(() => {
        // Async layout shifts (images loading, embeds expanding): re-pin to
        // bottom if we're in PINNED state. Suppressed during backward pagination
        // to avoid fighting with the scroll position adjustment in afterUpdate.
        const resizeObs = new ResizeObserver(() => {
            if (suppressResizeScroll) return;
            if (stuckAtBottom) scrollToBottom();
        });
        resizeObs.observe(contentElement);

        return () => resizeObs.disconnect();
    });

    // --- Scroll management in Svelte lifecycle ---

    beforeUpdate(() => {
        if (scrollerElement) {
            savedScrollHeight = scrollerElement.scrollHeight;
        }
    });

    afterUpdate(() => {
        if (!scrollerElement) return;

        const currentId = $activeChannel?.id;
        const entries = $activeWindow.entries;

        // Channel switch: always reset to PINNED and scroll to bottom.
        if (currentId !== prevChannelId) {
            prevChannelId = currentId;
            prevEntries = entries;
            stuckAtBottom = true;
            newMessagesPending = false;
            scrollToBottom();
            return;
        }

        // No data change — nothing to do.
        if (entries === prevEntries) return;

        const heightDelta = scrollerElement.scrollHeight - savedScrollHeight;

        if (stuckAtBottom) {
            // PINNED: any content change → stay at bottom
            scrollToBottom();
        } else if (heightDelta > 0) {
            // BROWSING: compensate for content added above viewport.
            // Exception: pure append (new message at bottom) → show badge instead.
            const isPureAppend = prevEntries.length > 0
                && entries[0] === prevEntries[0]
                && entries[entries.length - 1] !== prevEntries[prevEntries.length - 1];

            if (isPureAppend) {
                newMessagesPending = true;
            } else {
                suppressResizeScroll = true;
                programmaticScroll = true;
                scrollerElement.scrollBy(0, heightDelta);
                lastPrependTime = Date.now();
                requestAnimationFrame(() => { suppressResizeScroll = false; });
            }
        }

        prevEntries = entries;
    });

    // --- Display helpers ---

    function handleChatClick(event: MouseEvent) {
        const target = event.target as HTMLElement;
        if (target.tagName.toLowerCase() === 'img') {
            openImage((target as HTMLImageElement).src);
        }
    }

    function isMessage(kind: TimelineEntryKind): kind is { Message: ChatMessage } {
        return typeof kind === 'object' && 'Message' in kind;
    }

    function isStateEvent(kind: TimelineEntryKind): kind is { StateEvent: StateEventKind } {
        return typeof kind === 'object' && 'StateEvent' in kind;
    }

    function isDayDivider(kind: TimelineEntryKind): kind is { DayDivider: number } {
        return typeof kind === 'object' && 'DayDivider' in kind;
    }

    function formatDayDivider(timestamp: number): string {
        return new Intl.DateTimeFormat(undefined, {
            weekday: 'long',
            year: 'numeric',
            month: 'long',
            day: 'numeric',
        }).format(new Date(timestamp));
    }

    function stateEventText(kind: StateEventKind): string {
        if (typeof kind === 'string') return '';
        if ('RoomNameChanged' in kind) return `Room name changed to ${kind.RoomNameChanged.name}`;
        if ('RoomTopicChanged' in kind) return `Room topic changed to ${kind.RoomTopicChanged.topic}`;
        if ('RoomAvatarChanged' in kind) return 'Room avatar was changed';
        if ('MemberJoined' in kind) return `${kind.MemberJoined.user_id.split(':')[0]} joined the room`;
        if ('MemberLeft' in kind) return `${kind.MemberLeft.user_id.split(':')[0]} left the room`;
        if ('MemberInvited' in kind) return `${kind.MemberInvited.user_id.split(':')[0]} was invited`;
        if ('MemberBanned' in kind) return `${kind.MemberBanned.user_id.split(':')[0]} was banned`;
        return '';
    }

    const GROUP_THRESHOLD_MS = 5 * 60 * 1000;

    function isContinuation(entries: TimelineEntry[], index: number): boolean {
        if (index === 0) return false;
        const curr = entries[index];
        const prev = entries[index - 1];
        if (!isMessage(curr.kind) || !isMessage(prev.kind)) return false;
        const currMsg = curr.kind.Message;
        const prevMsg = prev.kind.Message;
        return currMsg.sender === prevMsg.sender
            && (currMsg.timestamp - prevMsg.timestamp) < GROUP_THRESHOLD_MS;
    }

    function entryKey(entry: TimelineEntry, index: number): string {
        if (isMessage(entry.kind)) return entry.kind.Message.id;
        return `entry-${index}`;
    }
</script>

<div class="chat-window">
    <header class="chat-header">
        {#if $activeChannel?.etch_room_type === 'Voice'}
            <svg class="header-icon" width="24" height="24" viewBox="0 0 24 24">
                <path fill="currentColor" d="M11.383 3.07904C11.009 2.92504 10.579 3.01004 10.293 3.29604L6 8.00204H3C2.45 8.00204 2 8.45304 2 9.00204V15.002C2 15.552 2.45 16.002 3 16.002H6L10.293 20.71C10.579 20.996 11.009 21.082 11.383 20.927C11.757 20.772 12 20.407 12 20.002V4.00204C12 3.59904 11.757 3.23204 11.383 3.07904ZM14 5.00195V7.00195C16.757 7.00195 19 9.24595 19 12.002C19 14.759 16.757 17.002 14 17.002V19.002C17.86 19.002 21 15.863 21 12.002C21 8.14295 17.86 5.00195 14 5.00195ZM14 9.00195C15.654 9.00195 17 10.349 17 12.002C17 13.657 15.654 15.002 14 15.002V13.002C14.551 13.002 15 12.553 15 12.002C15 11.451 14.551 11.002 14 11.002V9.00195Z"></path>
            </svg>
        {:else}
            <svg class="header-icon" width="24" height="24" viewBox="0 0 24 24">
                <path fill="currentColor" fill-rule="evenodd" clip-rule="evenodd" d="M5.88657 21C5.57547 21 5.3399 20.7189 5.39427 20.4126L6.00001 17H2.59511C2.28449 17 2.04905 16.7198 2.10259 16.4138L2.27759 15.4138C2.31946 15.1746 2.52722 15 2.77011 15H6.35001L7.41001 9H4.00511C3.69449 9 3.45905 8.71977 3.51259 8.41381L3.68759 7.41381C3.72946 7.17456 3.93722 7 4.18011 7H7.76001L8.39677 3.41262C8.43914 3.17391 8.64664 3 8.88907 3H9.87344C10.1845 3 10.4201 3.28107 10.3657 3.58738L9.76001 7H15.76L16.3968 3.41262C16.4391 3.17391 16.6466 3 16.8891 3H17.8734C18.1845 3 18.4201 3.28107 18.3657 3.58738L17.76 7H21.1649C21.4755 7 21.711 7.28023 21.6574 7.58619L21.4824 8.58619C21.4405 8.82544 21.2328 9 20.9899 9H17.41L16.35 15H19.7549C20.0655 15 20.301 15.2802 20.2474 15.5862L20.0724 16.5862C20.0305 16.8254 19.8228 17 19.5799 17H16L15.3632 20.5874C15.3209 20.8261 15.1134 21 14.8709 21H13.8866C13.5755 21 13.3399 20.7189 13.3943 20.4126L14 17H8.00001L7.36325 20.5874C7.32088 20.8261 7.11337 21 6.87094 21H5.88657ZM9.41045 9L8.35045 15H14.3504L15.4104 9H9.41045Z"></path>
            </svg>
        {/if}
        <h2>{$activeChannel?.display_name ?? ''}</h2>
        {#if $activeChannel?.is_encrypted}
            <svg class="lock-icon" width="16" height="16" viewBox="0 0 24 24">
                <path fill="currentColor" d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zM12 17c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zM15.1 8H8.9V6c0-1.71 1.39-3.1 3.1-3.1s3.1 1.39 3.1 3.1v2z"/>
            </svg>
        {/if}
        {#if $showRoomIds && $activeChannel}
            <span class="room-id" role="button" tabindex="0" title="Click to copy" on:click={() => {
                if ($activeChannel) navigator.clipboard.writeText($activeChannel.id);
            }} on:keydown={(e) => {
                if (e.key === 'Enter' && $activeChannel) navigator.clipboard.writeText($activeChannel.id);
            }}>{$activeChannel.id}</span>
        {/if}
    </header>

    <div class="messages-scroller" bind:this={scrollerElement} on:click={handleChatClick} on:scroll={onScroll}>
        {#if $activeWindow.loading}
            <div class="loading-indicator">Loading...</div>
        {/if}

        <div bind:this={contentElement}>
            {#each $activeWindow.entries as entry, i (entryKey(entry, i))}
                {#if isMessage(entry.kind)}
                    <MessageGroup
                        msg={entry.kind.Message}
                        sender={entry.sender}
                        continuation={isContinuation($activeWindow.entries, i)}
                    />
                {:else if isDayDivider(entry.kind)}
                    <div class="day-divider">
                        <span>{formatDayDivider(entry.kind.DayDivider)}</span>
                    </div>
                {:else if isStateEvent(entry.kind)}
                    {@const text = stateEventText(entry.kind.StateEvent)}
                    {#if text}
                        <div class="state-event">{text}</div>
                    {/if}
                {/if}
            {/each}
        </div>
    </div>

    {#if !stuckAtBottom}
        <button class="scroll-to-bottom" on:click={jumpToLatest}>
            {#if newMessagesPending}<span class="new-messages-dot"></span>{/if}
            <svg width="18" height="18" viewBox="0 0 24 24">
                <path fill="currentColor" d="M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6z"/>
            </svg>
        </button>
    {/if}
</div>

<style>
    .chat-window {
        display: flex;
        flex-direction: column;
        height: 100%;
        background-color: transparent;
        position: relative;
    }

    .chat-header {
        height: 48px;
        padding: 0 16px;
        display: flex;
        align-items: center;
        box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
        flex-shrink: 0;
        z-index: 2;
        color: #fff;
    }

    .header-icon { color: #8e9297; margin-right: 8px; }

    .chat-header h2 { font-size: 16px; font-weight: 600; margin: 0; }
    .lock-icon { color: #43b581; margin-left: 8px; flex-shrink: 0; }
    .room-id {
        margin-left: 10px;
        font-size: 12px;
        color: #72767d;
        font-family: monospace;
        cursor: pointer;
        user-select: none;
    }
    .room-id:hover { color: #dcddde; }
    .room-id:active { color: #43b581; }

    .messages-scroller {
        flex-grow: 1;
        overflow-y: scroll;
        overflow-x: hidden;
        overflow-anchor: none;
        padding: 16px 0;
        -webkit-user-select: text;
        user-select: text;
    }

    :global(.messages-scroller *) {
        -webkit-user-select: text;
        user-select: text;
    }

    .messages-scroller::-webkit-scrollbar { width: 8px; }
    .messages-scroller::-webkit-scrollbar-track { background: #2e3035; border-radius: 4px; margin-right: 4px; }
    .messages-scroller::-webkit-scrollbar-thumb { background-color: #202225; border-radius: 4px; }

    .loading-indicator {
        text-align: center;
        padding: 8px;
        font-size: 12px;
        color: #72767d;
    }

    .scroll-to-bottom {
        position: absolute;
        bottom: 12px;
        right: 20px;
        width: 36px;
        height: 36px;
        border-radius: 50%;
        background: #36393f;
        border: 1px solid #202225;
        color: #dcddde;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: center;
        box-shadow: 0 2px 8px rgba(0, 0, 0, 0.4);
        z-index: 3;
        transition: background-color 0.15s;
    }
    .scroll-to-bottom:hover {
        background: #40444b;
    }

    .new-messages-dot {
        position: absolute;
        top: -2px;
        right: -2px;
        width: 10px;
        height: 10px;
        border-radius: 50%;
        background: #5865f2;
    }

    /* --- Day divider --- */
    .day-divider {
        display: flex;
        align-items: center;
        margin: 16px 16px 8px;
    }

    .day-divider::before,
    .day-divider::after {
        content: '';
        flex: 1;
        height: 1px;
        background-color: #4f545c;
    }

    .day-divider span {
        padding: 0 8px;
        font-size: 12px;
        font-weight: 600;
        color: #72767d;
        white-space: nowrap;
    }

    /* --- State event --- */
    .state-event {
        padding: 4px 16px;
        font-size: 13px;
        font-style: italic;
        color: #72767d;
    }
</style>
