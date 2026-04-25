<script lang="ts">
    import { beforeUpdate, afterUpdate, onMount } from 'svelte';
    import { activeWindow, loadOlder, activeChannel, openImage, scrollSignal } from '$lib/stores';
    import type { ChatMessage, TimelineEntry, TimelineEntryKind, StateEventKind } from '$lib/types';
    import MessageGroup from './MessageGroup.svelte';

    let scrollerElement: HTMLDivElement;
    let sentinelElement: HTMLDivElement;

    let savedScrollHeight = 0;
    let prevChannelId: string | undefined = undefined;

    onMount(() => {
        const observer = new IntersectionObserver((entries) => {
            if (entries[0].isIntersecting && $activeWindow.hasMore && !$activeWindow.loading) {
                loadOlder();
            }
        }, { threshold: 0.1 });

        observer.observe(sentinelElement);
        return () => observer.disconnect();
    });

    beforeUpdate(() => {
        if (scrollerElement) {
            savedScrollHeight = scrollerElement.scrollHeight;
        }
    });

    afterUpdate(() => {
        if (!scrollerElement) return;

        const currentId = $activeChannel?.id;
        if (currentId !== prevChannelId) {
            prevChannelId = currentId;
            scrollerElement.scrollTop = scrollerElement.scrollHeight;
            scrollSignal.action = null;
            return;
        }

        const action = scrollSignal.action;
        if (action === 'prepend') {
            scrollerElement.scrollTop += scrollerElement.scrollHeight - savedScrollHeight;
        } else if (action === 'append') {
            scrollerElement.scrollTop = scrollerElement.scrollHeight;
        }
        scrollSignal.action = null;
    });

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

    const GROUP_THRESHOLD_MS = 5 * 60 * 1000; // 5 minutes

    // Check if this message should be grouped with the previous one (same sender, within threshold)
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

    // Unique key for each entry (for Svelte's keyed each block)
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
    </header>

    <div class="messages-scroller" bind:this={scrollerElement} on:click={handleChatClick}>
        <div bind:this={sentinelElement} class="scroll-sentinel"></div>

        {#if $activeWindow.loading}
            <div class="loading-indicator">Loading...</div>
        {/if}

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

<style>
    .chat-window {
        display: flex;
        flex-direction: column;
        height: 100%;
        background-color: transparent;
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

    .messages-scroller {
        flex-grow: 1;
        overflow-y: scroll;
        overflow-x: hidden;
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

    .scroll-sentinel { height: 1px; }

    .loading-indicator {
        text-align: center;
        padding: 8px;
        font-size: 12px;
        color: #72767d;
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
