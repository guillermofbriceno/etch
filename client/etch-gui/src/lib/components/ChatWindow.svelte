<script lang="ts">
    import { beforeUpdate, afterUpdate, onMount } from 'svelte';
    import { activeWindow, loadOlder, activeChannel, activeChannelId, openImage, openConnect, showRoomIds, encryptionStatus, unlockScreen, openEncryptionDialog, matrixStatus } from '$lib/stores';
    import { undecryptableLine } from '$lib/encryptionText';
    import type { ChatMessage, TimelineEntry, TimelineEntryKind, StateEventKind } from '$lib/types';
    import MessageGroup from './MessageGroup.svelte';
    import Icon from './Icon.svelte';
    import AvatarFallback from './AvatarFallback.svelte';
    import { resolveMediaUrl, getInitial } from '$lib/media';
    import { customScrollbar } from '$lib/scrollbar';

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
    let scrollTimeout: ReturnType<typeof setTimeout>;
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

        // Suppress child hover effects while scrolling so toolbars don't
        // appear when content moves under a stationary cursor.
        scrollerElement.classList.add('scrolling');
        clearTimeout(scrollTimeout);
        scrollTimeout = setTimeout(() => {
            scrollerElement.classList.remove('scrolling');
        }, 150);

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
        // A composer that grows takes its height from the scroller, which would cover the newest messages.
        resizeObs.observe(scrollerElement);

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

    function clickDelegate(node: HTMLElement) {
        function handleClick(event: MouseEvent) {
            const target = event.target as HTMLElement;
            if (target.tagName.toLowerCase() === 'img') {
                openImage((target as HTMLImageElement).src);
            }
        }
        node.addEventListener('click', handleClick);
        return { destroy() { node.removeEventListener('click', handleClick); } };
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

    function rendersNothing(kind: TimelineEntryKind): boolean {
        if (isStateEvent(kind)) return stateEventText(kind.StateEvent) === '';
        return kind === 'ReadMarker' || kind === 'Redacted' || kind === 'Other';
    }

    /** How many undecryptable entries the line at `index` stands for: the whole run for its first entry, 0 for the rest. */
    function undecryptableRun(entries: TimelineEntry[], index: number): number {
        for (let i = index - 1; i >= 0; i--) {
            const kind = entries[i].kind;
            if (kind === 'Undecryptable') return 0;
            if (!rendersNothing(kind)) break;
        }
        let count = 0;
        for (let i = index; i < entries.length; i++) {
            const kind = entries[i].kind;
            if (kind === 'Undecryptable') count++;
            else if (!rendersNothing(kind)) break;
        }
        return count;
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
            && (currMsg.timestamp - prevMsg.timestamp) < GROUP_THRESHOLD_MS
            && !currMsg.edited;
    }

    function entryKey(entry: TimelineEntry, index: number): string {
        if (isMessage(entry.kind)) return entry.kind.Message.id;
        return `entry-${index}`;
    }
</script>

<div class="chat-window">
    <header class="chat-header">
        {#if $activeChannel}
            {#if $activeChannel.etch_room_type === 'Voice'}
                <Icon name="volume" size={20} class="header-icon" />
            {:else if $activeChannel.etch_room_type !== 'Dm'}
                <Icon name="hash" size={20} class="header-icon" />
            {:else if $activeChannel.avatar_url}
                <img src={resolveMediaUrl($activeChannel.avatar_url)} alt="" class="header-avatar" />
            {:else}
                <AvatarFallback initial={getInitial($activeChannel.display_name)} size={24} />
            {/if}
            <h2>{$activeChannel.display_name}</h2>
            {#if $activeChannel.is_encrypted}
                <Icon name="lock" size={16} class="lock-icon" />
            {/if}
        {/if}
        {#if $showRoomIds && $activeChannel}
            <span class="room-id" role="button" tabindex="0" title="Click to copy" on:click={() => {
                if ($activeChannel) navigator.clipboard.writeText($activeChannel.id);
            }} on:keydown={(e) => {
                if (e.key === 'Enter' && $activeChannel) navigator.clipboard.writeText($activeChannel.id);
            }}>{$activeChannel.id}</span>
        {/if}
    </header>

    <div class="messages-scroller" bind:this={scrollerElement} use:clickDelegate use:customScrollbar on:scroll={onScroll}>
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
                        roomId={$activeChannelId ?? ''}
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
                {:else if entry.kind === 'Undecryptable'}
                    {@const count = undecryptableRun($activeWindow.entries, i)}
                    {#if count > 0}
                        {@const line = undecryptableLine(count, unlockScreen($encryptionStatus.type))}
                        {#if line.remedy}
                            <button class="undecryptable" on:click={openEncryptionDialog}>
                                {line.text} <span class="undecryptable-remedy">{line.remedy}</span>
                            </button>
                        {:else}
                            <div class="undecryptable">{line.text}</div>
                        {/if}
                    {/if}
                {/if}
            {/each}
        </div>
    </div>

    {#if $activeChannelId === null}
        <div class="empty-state">
            {#if $matrixStatus === 'disconnected'}
                <h3>Not connected to a server</h3>
                <p>Connect to a server to see its channels and messages.</p>
                <button class="empty-action" on:click={openConnect}>Connect to a Server</button>
            {:else if $matrixStatus === 'connecting'}
                <h3>Connecting to the server</h3>
                <p>Your channels will appear here in a moment.</p>
            {:else}
                <h3>No channel selected</h3>
                <p>Choose a channel from the sidebar.</p>
            {/if}
        </div>
    {/if}

    {#if !stuckAtBottom}
        <button class="scroll-to-bottom floating" aria-label="Jump to the latest message" on:click={jumpToLatest}>
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

    /* The margin matches the composer's, so the line under the header ends where the composer does. */
    .chat-header {
        box-sizing: border-box;
        height: 48px;
        margin: 0 10px;
        padding: 0 6px;
        display: flex;
        align-items: center;
        gap: 8px;
        border-bottom: 1px solid var(--border-input);
        flex-shrink: 0;
        z-index: 2;
        color: var(--text-bright);
    }

    .chat-header :global(.header-icon) { color: var(--text-tertiary); flex-shrink: 0; }
    .header-avatar { width: 24px; height: 24px; border-radius: 50%; object-fit: cover; flex-shrink: 0; }

    .chat-header h2 {
        font-size: 16px;
        font-weight: 600;
        margin: 0;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .chat-header :global(.lock-icon) { color: var(--status-success); flex-shrink: 0; }
    .room-id {
        margin-left: 2px;
        font-size: 12px;
        color: var(--text-muted);
        font-family: var(--font-family-mono);
        cursor: pointer;
        user-select: none;
    }
    .room-id:hover { color: var(--text-primary); }
    .room-id:active { color: var(--status-success); }

    .messages-scroller {
        flex-grow: 1;
        overflow-y: auto;
        overflow-x: hidden;
        overflow-anchor: none;
        padding: 16px 0;
        -webkit-user-select: text;
        user-select: text;
    }

    .messages-scroller:global(.scrolling) > :global(*) {
        pointer-events: none;
    }

    :global(.messages-scroller *) {
        -webkit-user-select: text;
        user-select: text;
    }

    .loading-indicator {
        text-align: center;
        padding: 8px;
        font-size: 12px;
        color: var(--text-muted);
    }

    .scroll-to-bottom {
        position: absolute;
        bottom: 12px;
        right: 20px;
        width: 36px;
        height: 36px;
        border-radius: 50%;
        color: var(--text-secondary);
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 3;
        transition: color 0.15s, border-color 0.15s;
    }
    .scroll-to-bottom:hover {
        color: var(--text-bright);
        border-color: var(--border-medium);
    }

    .empty-state {
        position: absolute;
        inset: 48px 0 0 0;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        padding: 24px;
        text-align: center;
    }

    .empty-state h3 { color: var(--text-bright); font-size: 18px; font-weight: 600; margin: 0 0 8px; }
    .empty-state p { color: var(--text-tertiary); font-size: var(--font-size-base); line-height: 1.4; margin: 0; }

    .empty-action {
        margin-top: 20px;
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

    .empty-action:hover { background-color: var(--primary-hover); }

    .new-messages-dot {
        position: absolute;
        top: -2px;
        right: -2px;
        width: 10px;
        height: 10px;
        border-radius: 50%;
        background: var(--primary);
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
        background-color: var(--border-input);
    }

    .day-divider span {
        padding: 0 8px;
        font-size: 12px;
        font-weight: 600;
        color: var(--text-muted);
        white-space: nowrap;
    }

    /* --- State event --- */
    .state-event {
        padding: 4px 16px;
        font-size: 13px;
        font-style: italic;
        color: var(--text-muted);
    }

    /* --- Undecryptable messages --- */
    .undecryptable {
        display: block;
        padding: 4px 16px;
        font-size: 13px;
        font-style: italic;
        color: var(--text-muted);
    }

    button.undecryptable {
        background: none;
        border: none;
        font-family: inherit;
        text-align: left;
        cursor: pointer;
    }

    .undecryptable-remedy {
        color: var(--text-link);
    }

    button.undecryptable:hover .undecryptable-remedy {
        text-decoration: underline;
    }
</style>
