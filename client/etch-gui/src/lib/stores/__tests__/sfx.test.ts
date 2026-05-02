import { describe, it, expect, beforeEach, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { playSfx, setSfxDeafened } from '../sfx';
import { deafenSuppressesNotifs } from '../voiceSettings';

beforeEach(() => {
    setSfxDeafened(false);
    deafenSuppressesNotifs.set(true);
    vi.mocked(invoke).mockClear();
});

describe('playSfx deafen gate', () => {
    it('plays any sound when not deafened', () => {
        playSfx('new_notif');
        playSfx('user_join');

        expect(invoke).toHaveBeenCalledTimes(2);
    });

    it('blocks all sounds when deafened (default setting)', () => {
        setSfxDeafened(true);

        playSfx('new_notif');
        playSfx('user_join');
        playSfx('mute');

        expect(invoke).not.toHaveBeenCalled();
    });

    it('allows new_notif through when deafened and suppression is off', () => {
        setSfxDeafened(true);
        deafenSuppressesNotifs.set(false);

        playSfx('new_notif');

        expect(invoke).toHaveBeenCalledTimes(1);
        expect(invoke).toHaveBeenCalledWith('play_sfx', expect.objectContaining({ name: 'new_notif' }));
    });

    it('still blocks non-notif sounds when deafened and suppression is off', () => {
        setSfxDeafened(true);
        deafenSuppressesNotifs.set(false);

        playSfx('user_join');
        playSfx('mute');
        playSfx('server_join');

        expect(invoke).not.toHaveBeenCalled();
    });
});
