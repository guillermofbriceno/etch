import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { nameColors, colorOf, chosenColorOf, handleMatrixEvent } from '../nameColors';
import { resetMatrixSession } from '../session';
import type { NameColor } from '$lib/types';
import { resetStores } from './helpers';

const ALICE = '@alice:example.org';
const BOB = '@bob:example.org';
const CAROL = '@carol:example.org';

beforeEach(() => {
    resetStores();
    vi.mocked(invoke).mockClear();
});

function nextTick(): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve));
}

function resolveRequest(userIds: string[]) {
    return ['core_command', { command: { type: 'Matrix', data: { type: 'ResolveNameColors', data: userIds } } }];
}

function answer(...entries: [string, NameColor | null][]): void {
    handleMatrixEvent({ type: 'NameColors', data: entries.map(([user_id, color]) => ({ user_id, color })) });
}

describe('resolving name colors on demand', () => {
    it('batches every miss in one tick into a single request', async () => {
        const colors = get(nameColors);
        colorOf(colors, ALICE);
        colorOf(colors, BOB);
        chosenColorOf(colors, CAROL);

        expect(invoke).not.toHaveBeenCalled();
        await nextTick();

        expect(vi.mocked(invoke).mock.calls).toEqual([resolveRequest([ALICE, BOB, CAROL])]);
    });

    it('never requests a user whose answer is known or already on its way', async () => {
        answer([ALICE, { color: '#ff8800' }], [BOB, null]);

        colorOf(get(nameColors), ALICE);
        colorOf(get(nameColors), BOB);
        colorOf(get(nameColors), CAROL);
        await nextTick();
        colorOf(get(nameColors), CAROL);
        await nextTick();

        expect(vi.mocked(invoke).mock.calls).toEqual([resolveRequest([CAROL])]);
    });

    it('requests an unanswered user again after a Matrix session reset', async () => {
        colorOf(new Map(), ALICE);
        await nextTick();

        resetMatrixSession();
        colorOf(new Map(), ALICE);
        await nextTick();

        expect(vi.mocked(invoke).mock.calls).toEqual([resolveRequest([ALICE]), resolveRequest([ALICE])]);
    });
});

describe('handleMatrixEvent (nameColors)', () => {
    it('records each answer, including no color, and keeps every color it does not mention', () => {
        answer([ALICE, { color: '#ff8800' }], [BOB, { color: '#62baf7' }]);

        answer([ALICE, null], [CAROL, { color: '#51caa4' }]);

        expect(get(nameColors)).toEqual(new Map([[ALICE, null], [BOB, { color: '#62baf7' }], [CAROL, { color: '#51caa4' }]]));
    });
});
