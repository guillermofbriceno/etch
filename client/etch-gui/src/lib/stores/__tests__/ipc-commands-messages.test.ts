import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { sendMessage, createDirectMessage, toggleReaction, loadOlder, setActiveChannel, activeWindow } from '../messages';
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
        await sendMessage('room1', 'hello', '<p>hello</p>');

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: {
                    type: 'SendMessage',
                    data: { room_id: 'room1', text: 'hello', html_body: '<p>hello</p>' },
                },
            },
        });
    });

    it('sendMessage defaults html_body to null', async () => {
        await sendMessage('room1', 'plain text');

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: {
                    type: 'SendMessage',
                    data: { room_id: 'room1', text: 'plain text', html_body: null },
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

    it('setActiveChannel updates the activeChannelId store', () => {
        setActiveChannel('new-room');

        expect(get(activeChannelId)).toBe('new-room');
    });
});
