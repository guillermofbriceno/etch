import { describe, it, expect, beforeEach } from 'vitest';
import { get } from 'svelte/store';
import { resetStores } from './helpers';
import { matrixStatus, matrixConnecting, matrixSessionLive, handleMatrixEvent } from '../matrixConnection';
import { activeOverlay } from '../overlay';

beforeEach(() => {
    resetStores();
});

// That a reset returns the status to disconnected is covered by its probe in session.test.ts.
describe('the Matrix connection status', () => {
    it('has a session to act on only once a connect has landed, and closes the overlay it was started from', () => {
        activeOverlay.set('connect');

        handleMatrixEvent({ type: 'ConnectionState', data: { type: 'Connecting' } });
        expect([get(matrixConnecting), get(matrixSessionLive)]).toEqual([true, false]);
        expect(get(activeOverlay)).toBe('connect');

        handleMatrixEvent({ type: 'ConnectionState', data: { type: 'Connected' } });
        expect([get(matrixConnecting), get(matrixSessionLive)]).toEqual([false, true]);
        expect(get(activeOverlay)).toBe('none');
    });

    // Core reports a degraded sync as Connecting and Connected, with no reset before it.
    it('keeps the session, and whatever is open, through a sync that degrades and recovers', () => {
        handleMatrixEvent({ type: 'ConnectionState', data: { type: 'Connected' } });
        activeOverlay.set('settings');

        handleMatrixEvent({ type: 'ConnectionState', data: { type: 'Connecting' } });
        expect(get(matrixStatus)).toBe('recovering');
        expect(get(matrixSessionLive), 'core still accepts requests while it retries the sync').toBe(true);

        handleMatrixEvent({ type: 'ConnectionState', data: { type: 'Connected' } });
        expect(get(matrixStatus)).toBe('connected');
        expect(get(activeOverlay), 'a blip must cost the user nothing visible').toBe('settings');
    });

    it('has no session after a failure', () => {
        handleMatrixEvent({ type: 'ConnectionState', data: { type: 'Connected' } });

        handleMatrixEvent({ type: 'ConnectionState', data: { type: 'Failed', reason: 'gone', retries: 1, retry_in_secs: 2 } });

        expect(get(matrixSessionLive)).toBe(false);
    });
});
