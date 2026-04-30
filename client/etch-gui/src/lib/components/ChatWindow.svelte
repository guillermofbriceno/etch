<script lang="ts">
    import { beforeUpdate, afterUpdate, onMount } from 'svelte';
    import { activeWindow, loadOlder, activeChannel, openImage, showRoomIds } from '$lib/stores';
    import type { ChatMessage, TimelineEntry, TimelineEntryKind, StateEventKind } from '$lib/types';
    import MessageGroup from './MessageGroup.svelte';
    import Icon from './Icon.svelte';

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

    const dayFormatter = new Intl.DateTimeFormat(undefined, {
        weekday: 'long',
        year: 'numeric',
        month: 'long',
        day: 'numeric',
    });

    function formatDayDivider(timestamp: number): string {
        return dayFormatter.format(new Date(timestamp));
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
            <Icon name="volume" class="header-icon" />
        {:else}
            <Icon name="hash" class="header-icon" />
        {/if}
        <h2>{$activeChannel?.display_name ?? ''}</h2>
        {#if $activeChannel?.is_encrypted}
            <Icon name="lock" size={16} class="lock-icon" />
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
            <Icon name="chevron_down" size={18} />
        </button>
    {/if}
</div>

<style>
    .chat-window {
        display: flex;
        flex-direction: column;
        height: 100%;
        min-width: 0;
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

    .chat-header :global(.header-icon) { color: #8e9297; margin-right: 8px; }

    .chat-header h2 { font-size: 16px; font-weight: 600; margin: 0; }
    .chat-header :global(.lock-icon) { color: #43b581; margin-left: 8px; flex-shrink: 0; }
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
