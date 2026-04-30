import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { sendMessage, createDirectMessage, toggleReaction, loadOlder, setActiveChannel, handleMatrixEvent, activeWindow } from '../messages';
import { activeChannelId } from '../activeChannel';
import { resetStores } from './helpers';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

beforeEach(() => {
    resetStores();
    vi.mocked(invoke).mockClear();
});

describe('messages IPC commands', () => {
    it('sendMessage sends Matrix > SendMessage', async () => {
        await sendMessage('room1', 'hello', '<p>hello</p>', '/tmp/file.png');

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: {
                    type: 'SendMessage',
                    data: { room_id: 'room1', text: 'hello', html_body: '<p>hello</p>', attachment_path: '/tmp/file.png' },
                },
            },
        });
    });

    it('sendMessage defaults html_body and attachment_path to null', async () => {
        await sendMessage('room1', 'plain text');

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: {
                    type: 'SendMessage',
                    data: { room_id: 'room1', text: 'plain text', html_body: null, attachment_path: null },
                },
            },
        });
    });

    it('createDirectMessage sends Matrix > CreateDirectMessage', async () => {
        await createDirectMessage('@bob:etch.gg');

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: { type: 'CreateDirectMessage', data: { target_user_id: '@bob:etch.gg' } },
            },
        });
    });

    it('toggleReaction sends Matrix > ToggleReaction with active channel', async () => {
        activeChannelId.set('room1');

        await toggleReaction('$event1', '👍');

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: { type: 'ToggleReaction', data: { room_id: 'room1', event_id: '$event1', key: '👍' } },
            },
        });
    });

    it('toggleReaction does nothing when no channel is active', async () => {
        await toggleReaction('$event1', '👍');

        expect(invoke).not.toHaveBeenCalled();
    });

    it('loadOlder sends Matrix > PaginateBackwards', () => {
        activeChannelId.set('room1');

        loadOlder();

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: { type: 'PaginateBackwards', data: { room_id: 'room1' } },
            },
        });
        expect(get(activeWindow).loading).toBe(true);
    });

    it('loadOlder does nothing when no channel is active', () => {
        loadOlder();

        expect(invoke).not.toHaveBeenCalled();
    });

    it('setActiveChannel sends Matrix > SendReadReceipt for the last message', () => {
        activeChannelId.set('room1');

        // Seed a message so there's something to send a receipt for
        handleMatrixEvent({
            type: 'TimelinePushBack',
            data: ['room1', {
                sender: { display_name: 'A', avatar_url: null },
                kind: { Message: { id: '$msg1', sender: '@a:s', body: 'hi', html_body: null, media: null, timestamp: Date.now(), reactions: {} } },
            }],
        } as any);
        vi.mocked(invoke).mockClear();

        setActiveChannel('room1');

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: { type: 'SendReadReceipt', data: { room_id: 'room1', event_id: '$msg1' } },
            },
        });
    });

    it('setActiveChannel does not send read receipt when channel has no messages', () => {
        vi.mocked(invoke).mockClear();

        setActiveChannel('empty-room');

        // It will set activeChannelId and ensureWindowExists, but no receipt
        const receiptCalls = vi.mocked(invoke).mock.calls.filter(
            c => c[1] && (c[1] as any).command?.data?.type === 'SendReadReceipt'
        );
        expect(receiptCalls).toHaveLength(0);
    });

    it('setActiveChannel skips non-Message entries to find the last Message', () => {
        activeChannelId.set('room1');

        // Seed a Message followed by a non-Message entry
        handleMatrixEvent({
            type: 'TimelinePushBack',
            data: ['room1', {
                sender: { display_name: 'A', avatar_url: null },
                kind: { Message: { id: '$msg1', sender: '@a:s', body: 'hi', html_body: null, media: null, timestamp: Date.now(), reactions: {} } },
            }],
        } as any);
        handleMatrixEvent({
            type: 'TimelinePushBack',
            data: ['room1', {
                sender: null,
                kind: 'ReadMarker',
            }],
        } as any);
        vi.mocked(invoke).mockClear();

        setActiveChannel('room1');

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: { type: 'SendReadReceipt', data: { room_id: 'room1', event_id: '$msg1' } },
            },
        });
    });

    it('setActiveChannel picks the last Message when multiple messages exist', () => {
        activeChannelId.set('room1');

        handleMatrixEvent({
            type: 'TimelinePushBack',
            data: ['room1', {
                sender: { display_name: 'A', avatar_url: null },
                kind: { Message: { id: '$msg1', sender: '@a:s', body: 'first', html_body: null, media: null, timestamp: Date.now(), reactions: {} } },
            }],
        } as any);
        handleMatrixEvent({
            type: 'TimelinePushBack',
            data: ['room1', {
                sender: { display_name: 'B', avatar_url: null },
                kind: { Message: { id: '$msg2', sender: '@b:s', body: 'second', html_body: null, media: null, timestamp: Date.now(), reactions: {} } },
            }],
        } as any);
        vi.mocked(invoke).mockClear();

        setActiveChannel('room1');

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: { type: 'SendReadReceipt', data: { room_id: 'room1', event_id: '$msg2' } },
            },
        });
    });

    it('setActiveChannel updates the activeChannelId store', () => {
        setActiveChannel('new-room');

        expect(get(activeChannelId)).toBe('new-room');
    });
});
