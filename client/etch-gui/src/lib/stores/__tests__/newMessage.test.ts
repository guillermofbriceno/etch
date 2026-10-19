import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { handleMatrixEvent } from '../newMessage';
import { activeChannelId } from '../activeChannel';
import { appFocused } from '../eventRouter';
import { resetStores } from './helpers';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

import * as sfx from '../sfx';

describe('notification sounds', () => {
    const ROOM = 'notif-room';

    // The cooldown's last-played time is private to the module and outlives a test, so
    // each test starts 30s after the previous one.
    let fakeTime = Date.UTC(2099, 0, 1);

    function receive(roomId: string, body: string) {
        handleMatrixEvent({ type: 'NewMessage', data: { room_id: roomId, sender_name: 'Other', body } });
    }

    beforeEach(() => {
        resetStores();
        vi.mocked(sfx.playSfx).mockClear();
        fakeTime += 30_000;
        vi.useFakeTimers({ now: fakeTime });
        activeChannelId.set(ROOM);
        appFocused.set(true);
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it('does not play sound when app is focused and message is in the active room', () => {
        receive(ROOM, 'hello');

        expect(vi.mocked(sfx.playSfx)).not.toHaveBeenCalledWith('new_notif');
    });

    it('plays sound when message is in a non-active room', () => {
        activeChannelId.set('other-room');

        receive(ROOM, 'hello');

        expect(vi.mocked(sfx.playSfx)).toHaveBeenCalledWith('new_notif');
    });

    it('plays sound when app is unfocused even in the active room', () => {
        appFocused.set(false);

        receive(ROOM, 'hello');

        expect(vi.mocked(sfx.playSfx)).toHaveBeenCalledWith('new_notif');
    });

    it('respects the 20-second cooldown between notification sounds', () => {
        appFocused.set(false);

        receive(ROOM, 'first');
        expect(vi.mocked(sfx.playSfx)).toHaveBeenCalledWith('new_notif');
        vi.mocked(sfx.playSfx).mockClear();

        vi.advanceTimersByTime(10_000);
        receive(ROOM, 'second');
        expect(vi.mocked(sfx.playSfx)).not.toHaveBeenCalledWith('new_notif');

        vi.advanceTimersByTime(10_000);
        receive(ROOM, 'third');
        expect(vi.mocked(sfx.playSfx)).toHaveBeenCalledWith('new_notif');
    });
});
