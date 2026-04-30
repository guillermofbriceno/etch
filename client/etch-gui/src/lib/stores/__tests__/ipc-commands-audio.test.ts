import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { toggleMute, toggleDeafen, isMuted, isDeafened } from '../audio';
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

describe('audio IPC commands', () => {
    describe('toggleMute', () => {
        it('sends System > MuteMic true when unmuted', async () => {
            await toggleMute();

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'System', data: { type: 'MuteMic', data: true } },
            });
        });

        it('sets isMuted to true when toggling from unmuted', async () => {
            await toggleMute();

            expect(get(isMuted)).toBe(true);
        });

        it('sends System > MuteMic false when muted', async () => {
            isMuted.set(true);

            await toggleMute();

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'System', data: { type: 'MuteMic', data: false } },
            });
        });

        it('sets isMuted to false when toggling from muted', async () => {
            isMuted.set(true);

            await toggleMute();

            expect(get(isMuted)).toBe(false);
        });

        it('while deafened sends System > Deafen false (undeafen)', async () => {
            isDeafened.set(true);

            await toggleMute();

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'System', data: { type: 'Deafen', data: false } },
            });
        });

        it('while deafened clears both isDeafened and isMuted', async () => {
            isDeafened.set(true);
            isMuted.set(true);

            await toggleMute();

            expect(get(isDeafened)).toBe(false);
            expect(get(isMuted)).toBe(false);
        });

        it('while deafened does not send MuteMic (only Deafen)', async () => {
            isDeafened.set(true);

            await toggleMute();

            const calls = vi.mocked(invoke).mock.calls;
            const muteMicCalls = calls.filter(
                c => c[1] && (c[1] as any).command?.data?.type === 'MuteMic'
            );
            expect(muteMicCalls).toHaveLength(0);
        });

        it('double toggle returns to original state', async () => {
            await toggleMute();
            expect(get(isMuted)).toBe(true);

            await toggleMute();
            expect(get(isMuted)).toBe(false);
        });
    });

    describe('toggleDeafen', () => {
        it('sends System > Deafen true when undeafened', async () => {
            await toggleDeafen();

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'System', data: { type: 'Deafen', data: true } },
            });
        });

        it('sets isDeafened to true when toggling from undeafened', async () => {
            await toggleDeafen();

            expect(get(isDeafened)).toBe(true);
        });

        it('also sets isMuted to true when deafening', async () => {
            await toggleDeafen();

            expect(get(isMuted)).toBe(true);
        });

        it('sends System > Deafen false when deafened', async () => {
            isDeafened.set(true);

            await toggleDeafen();

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'System', data: { type: 'Deafen', data: false } },
            });
        });

        it('sets isDeafened to false and isMuted to false when undeafening', async () => {
            isDeafened.set(true);
            isMuted.set(true);

            await toggleDeafen();

            expect(get(isDeafened)).toBe(false);
            expect(get(isMuted)).toBe(false);
        });

        it('double toggle returns to original state', async () => {
            await toggleDeafen();
            expect(get(isDeafened)).toBe(true);

            await toggleDeafen();
            expect(get(isDeafened)).toBe(false);
            expect(get(isMuted)).toBe(false);
        });
    });
});
