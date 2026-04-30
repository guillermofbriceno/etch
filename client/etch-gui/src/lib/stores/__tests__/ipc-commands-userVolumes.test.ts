import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { userVolumes, setUserVolume } from '../userVolumes';
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

describe('userVolumes IPC commands', () => {
    it('setUserVolume sends Mumble > SetUserVolume', () => {
        setUserVolume(10, -5.0);

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Mumble',
                data: { type: 'SetUserVolume', data: { session_id: 10, volume_db: -5.0 } },
            },
        });
    });

    it('updates the userVolumes store', () => {
        setUserVolume(10, -3.0);

        expect(get(userVolumes)[10]).toBe(-3.0);
    });

    it('tracks volumes for multiple users independently', () => {
        setUserVolume(10, -3.0);
        setUserVolume(20, 6.0);

        expect(get(userVolumes)[10]).toBe(-3.0);
        expect(get(userVolumes)[20]).toBe(6.0);
    });

    it('overwrites a previous volume for the same user', () => {
        setUserVolume(10, -3.0);
        setUserVolume(10, 2.0);

        expect(get(userVolumes)[10]).toBe(2.0);
    });

    it('handles zero volume', () => {
        setUserVolume(10, 0);

        expect(get(userVolumes)[10]).toBe(0);
        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Mumble',
                data: { type: 'SetUserVolume', data: { session_id: 10, volume_db: 0 } },
            },
        });
    });
});
