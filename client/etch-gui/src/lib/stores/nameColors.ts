import { writable, get } from 'svelte/store';
import type { MatrixEvent } from '$lib/ipc';
import { sendCoreCommand } from '$lib/ipc';
import type { NameColor } from '$lib/types';
import { userColor } from '$lib/userColor';
import { registerSessionStore } from './session';

// An absent user has not been asked about yet; null means they have no color.
export const nameColors = writable<Map<string, NameColor | null>>(new Map());
export const canSetNameColor = writable(false);

const requested = new Set<string>();
let batch: string[] = [];

export function resetNameColors(): void {
    nameColors.set(new Map());
    requested.clear();
    batch = [];
}

const resetCanSetNameColor = (): void => { canSetNameColor.set(false); };

registerSessionStore('matrix', 'nameColors', resetNameColors);
registerSessionStore('matrix', 'canSetNameColor', resetCanSetNameColor);

/** Takes the map so a caller passing `$nameColors` re-renders when it changes; a miss asks core for the user. */
export function chosenColorOf(colors: Map<string, NameColor | null>, userId: string): NameColor | null {
    const chosen = colors.get(userId);
    if (chosen === undefined) request(userId);
    return chosen ?? null;
}

export function colorOf(colors: Map<string, NameColor | null>, userId: string): string {
    return userColor(userId, chosenColorOf(colors, userId));
}

function request(userId: string): void {
    if (requested.has(userId)) return;
    requested.add(userId);
    if (batch.push(userId) === 1) queueMicrotask(flush);
}

function flush(): void {
    // A reset between scheduling and now leaves nothing to send.
    if (batch.length === 0) return;
    const userIds = batch;
    batch = [];
    sendCoreCommand({ type: 'Matrix', data: { type: 'ResolveNameColors', data: userIds } });
}

export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type === 'NameColors') {
        const colors = new Map(get(nameColors));
        for (const { user_id, color } of me.data) {
            colors.set(user_id, color);
            requested.delete(user_id);
        }
        nameColors.set(colors);
    } else if (me.type === 'Capabilities') {
        canSetNameColor.set(me.data.name_color);
    }
}
