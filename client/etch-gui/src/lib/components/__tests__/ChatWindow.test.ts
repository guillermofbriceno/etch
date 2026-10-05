import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { tick } from 'svelte';
import { get } from 'svelte/store';
import { resetStores } from '$lib/stores/__tests__/helpers';
import { activeChannelId } from '$lib/stores/activeChannel';
import { handleMatrixEvent as handleTimelineEvent } from '$lib/stores/messages';
import {
    handleMatrixEvent as handleEncryptionEvent, encryptionPromptDismissed, dismissEncryptionPrompt,
} from '$lib/stores/encryption';
import type { EncryptionStatus, TimelineEntry, TimelineEntryKind } from '$lib/types';
import ChatWindow from '../ChatWindow.svelte';

vi.mock('$lib/markdown', () => ({
    markdownToHtml: vi.fn((md: string) => `<p>${md}</p>`),
}));

vi.mock('$lib/highlight', () => ({
    hljs: { highlightElement: vi.fn() },
}));

const ROOM = '!room:test';

function entry(kind: TimelineEntryKind): TimelineEntry {
    return { sender: null, kind };
}

function message(id: string, body: string): TimelineEntry {
    return {
        sender: { display_name: 'Alice', avatar_url: null },
        kind: {
            Message: {
                id, sender: '@alice:test', body, html_body: null, media: null,
                timestamp: 1_700_000_000_000, edited: false, reactions: {},
            },
        },
    };
}

const undecryptable = (): TimelineEntry => entry('Undecryptable');

async function report(status: EncryptionStatus): Promise<void> {
    handleEncryptionEvent({ type: 'EncryptionStatus', data: status });
    await tick();
}

async function show(entries: TimelineEntry[], status: EncryptionStatus): Promise<void> {
    activeChannelId.set(ROOM);
    handleTimelineEvent({ type: 'TimelineReset', data: [ROOM, entries] });
    render(ChatWindow);
    await report(status);
}

function notices(): string[] {
    return screen.queryAllByText(/encrypted message/).map((line) => line.textContent ?? '');
}

beforeEach(() => {
    resetStores();
    handleTimelineEvent({ type: 'TimelineCleared', data: ROOM });
});

describe('undecryptable messages in the chat window', () => {
    it('stands a run of them in as one line with their count', async () => {
        await show([
            undecryptable(),
            undecryptable(),
            entry('ReadMarker'),
            undecryptable(),
            message('$visible', 'a message that splits the run'),
            undecryptable(),
        ], { type: 'Ready' });

        expect(notices(), 'an entry that shows nothing must not split a run, and a message must').toEqual([
            '3 encrypted messages cannot be read on this device.',
            '1 encrypted message cannot be read on this device.',
        ]);
    });

    it('asks for the recovery key only on a device that needs it', async () => {
        const user = userEvent.setup();
        await show([undecryptable(), undecryptable()], { type: 'NeedsRecoveryKey' });
        dismissEncryptionPrompt();

        await user.click(screen.getByRole('button', {
            name: '2 encrypted messages. Enter your recovery key to read them.',
        }));
        expect(get(encryptionPromptDismissed), 'the line should bring the dismissed prompt back').toBe(false);

        await report({ type: 'Ready' });
        expect(notices()).toEqual(['2 encrypted messages cannot be read on this device.']);
        expect(
            screen.queryByRole('button', { name: /encrypted message/ }),
            'no key would help here, so there is nothing to open',
        ).toBeNull();
    });

    it('gives way to the message once it can be read', async () => {
        await show([undecryptable(), undecryptable()], { type: 'NeedsRecoveryKey' });
        expect(notices()).toEqual(['2 encrypted messages. Enter your recovery key to read them.']);

        handleTimelineEvent({ type: 'TimelineSet', data: [ROOM, 0, message('$first', 'now readable')] });
        await report({ type: 'Ready' });

        expect(screen.getByText('now readable')).toBeInTheDocument();
        expect(notices()).toEqual(['1 encrypted message cannot be read on this device.']);
    });
});
