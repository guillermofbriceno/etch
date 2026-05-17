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
        setUserVolume('alice', 10, -5.0);

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Mumble',
                data: { type: 'SetUserVolume', data: { session_id: 10, volume_db: -5.0 } },
            },
        });
    });

    it('updates the userVolumes store', () => {
        setUserVolume('alice', 10, -3.0);

        expect(get(userVolumes)['alice']).toBe(-3.0);
    });

    it('tracks volumes for multiple users independently', () => {
        setUserVolume('alice', 10, -3.0);
        setUserVolume('bob', 20, 6.0);

        expect(get(userVolumes)['alice']).toBe(-3.0);
        expect(get(userVolumes)['bob']).toBe(6.0);
    });

    it('overwrites a previous volume for the same user', () => {
        setUserVolume('alice', 10, -3.0);
        setUserVolume('alice', 10, 2.0);

        expect(get(userVolumes)['alice']).toBe(2.0);
    });

    it('handles zero volume', () => {
        setUserVolume('alice', 10, 0);

        expect(get(userVolumes)['alice']).toBe(0);
        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Mumble',
                data: { type: 'SetUserVolume', data: { session_id: 10, volume_db: 0 } },
            },
        });
    });

    it('persists volume across session_id changes', () => {
        setUserVolume('alice', 10, -5.0);
        // Alice reconnects with new session_id
        setUserVolume('alice', 42, 3.0);

        expect(get(userVolumes)['alice']).toBe(3.0);
        expect(invoke).toHaveBeenLastCalledWith('core_command', {
            command: {
                type: 'Mumble',
                data: { type: 'SetUserVolume', data: { session_id: 42, volume_db: 3.0 } },
            },
        });
    });
});
