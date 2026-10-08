import { writable, derived, get } from 'svelte/store';
import type { MatrixEvent } from '$lib/ipc';
import { closeOverlay } from './overlay';
import { registerSessionStore, declareStores } from './session';

// `recovering` is a degraded sync: the session is still there to act on while core retries.
export type MatrixStatus = 'disconnected' | 'connecting' | 'connected' | 'recovering';

export const matrixStatus = writable<MatrixStatus>('disconnected');
export const matrixConnecting = derived(matrixStatus, ($s) => $s === 'connecting' || $s === 'recovering');
export const matrixSessionLive = derived(matrixStatus, ($s) => $s === 'connected' || $s === 'recovering');

registerSessionStore('matrix', 'matrixStatus', () => { matrixStatus.set('disconnected'); });
declareStores('derived', 'matrixConnecting', 'matrixSessionLive');

type ReportedState = Extract<MatrixEvent, { type: 'ConnectionState' }>['data']['type'];

function matrixStatusAfter(before: MatrixStatus, reported: ReportedState): MatrixStatus {
    switch (reported) {
        case 'Connected': return 'connected';
        // Core sends a reset ahead of every connect attempt, so Connecting on a live session is a sync that degraded.
        case 'Connecting': return before === 'connected' || before === 'recovering' ? 'recovering' : 'connecting';
        default: return 'disconnected';
    }
}

export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type !== 'ConnectionState') return;
    const before = get(matrixStatus);
    const after = matrixStatusAfter(before, me.data.type);
    matrixStatus.set(after);
    // A connect that succeeded, not a sync that recovered.
    if (after === 'connected' && before !== 'recovering' && before !== 'connected') closeOverlay();
}
