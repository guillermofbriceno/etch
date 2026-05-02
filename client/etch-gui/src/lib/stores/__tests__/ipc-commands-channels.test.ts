import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { channels, hideDm, unhideDm } from '../channels';
import { activeChannelId } from '../activeChannel';
import { resetStores } from './helpers';
import type { RoomInfo } from '$lib/types';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

beforeEach(() => {
    resetStores();
    vi.mocked(invoke).mockClear();
});

function makeRoom(id: string, name: string, type: 'Voice' | 'Text' | 'Dm' = 'Text'): RoomInfo {
    return { id, display_name: name, etch_room_type: type, channel_id: null, is_default: false, unread_count: 0, is_encrypted: false, avatar_url: null };
}

describe('channels IPC commands', () => {
    describe('hideDm', () => {
        it('sends System > HideDm', () => {
            channels.set([makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')]);
            activeChannelId.set('t1');
            vi.mocked(invoke).mockClear();

            hideDm('dm1');

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'System', data: { type: 'HideDm', data: { room_id: 'dm1' } } },
            });
        });

        it('removes the DM from the channels store', () => {
            channels.set([makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')]);
            activeChannelId.set('t1');

            hideDm('dm1');

            expect(get(channels).find(c => c.id === 'dm1')).toBeUndefined();
            expect(get(channels)).toHaveLength(1);
        });

        it('switches active channel when hiding the active DM', () => {
            channels.set([makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')]);
            activeChannelId.set('dm1');

            hideDm('dm1');

            expect(get(activeChannelId)).not.toBe('dm1');
        });

        it('does not switch active channel when hiding a non-active DM', () => {
            channels.set([makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')]);
            activeChannelId.set('t1');

            hideDm('dm1');

            expect(get(activeChannelId)).toBe('t1');
        });

        it('prefers Voice channel as fallback when hiding active DM', () => {
            channels.set([
                makeRoom('dm1', 'Alice', 'Dm'),
                makeRoom('t1', 'General'),
                makeRoom('v1', 'Voice', 'Voice'),
            ]);
            activeChannelId.set('dm1');

            hideDm('dm1');

            expect(get(activeChannelId)).toBe('v1');
        });
    });

    describe('unhideDm', () => {
        it('sends System > UnhideDm', () => {
            channels.set([makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')]);
            activeChannelId.set('t1');
            hideDm('dm1');
            vi.mocked(invoke).mockClear();

            unhideDm('dm1');

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'System', data: { type: 'UnhideDm', data: { room_id: 'dm1' } } },
            });
        });

        it('restores the DM to the channels store', () => {
            channels.set([makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')]);
            activeChannelId.set('t1');
            hideDm('dm1');

            unhideDm('dm1');

            expect(get(channels).find(c => c.id === 'dm1')).toBeDefined();
        });

        it('restores the DM with unread_count of 1', () => {
            channels.set([makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')]);
            activeChannelId.set('t1');
            hideDm('dm1');

            unhideDm('dm1');

            expect(get(channels).find(c => c.id === 'dm1')?.unread_count).toBe(1);
        });

        it('is a no-op when the DM is not hidden', () => {
            channels.set([makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')]);
            activeChannelId.set('t1');
            vi.mocked(invoke).mockClear();

            unhideDm('dm1');

            // Should not send any command since it's not hidden
            const unhideCalls = vi.mocked(invoke).mock.calls.filter(
                c => c[1] && (c[1] as any).command?.data?.type === 'UnhideDm'
            );
            expect(unhideCalls).toHaveLength(0);
        });

        it('does not duplicate if unhidden twice', () => {
            channels.set([makeRoom('dm1', 'Alice', 'Dm'), makeRoom('t1', 'General')]);
            activeChannelId.set('t1');
            hideDm('dm1');

            unhideDm('dm1');
            unhideDm('dm1'); // second call should be a no-op

            expect(get(channels).filter(c => c.id === 'dm1')).toHaveLength(1);
        });
    });
});
