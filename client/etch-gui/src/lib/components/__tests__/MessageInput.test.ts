import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { activeChannelId, replyingTo, setReply, clearReply } from '$lib/stores';
import { handleMatrixEvent } from '$lib/stores/messages';
import type { ChatMessage } from '$lib/types';
import { resetStores } from '$lib/stores/__tests__/helpers';
import MessageInput from '../MessageInput.svelte';

vi.mock('$lib/markdown', () => ({
    composeHtml: vi.fn((md: string) => `<p>${md}</p>\n`),
    insertMentionLinks: vi.fn((html: string) => html),
}));

const ROOM = 'room1';

/** Push a Message entry into the timeline for the given room. */
function seedUser(roomId: string, sender: string, displayName: string) {
    handleMatrixEvent({
        type: 'TimelinePushBack',
        data: [roomId, {
            sender: { display_name: displayName, avatar_url: null },
            kind: { Message: { id: `$${sender}`, sender, body: 'hi', html_body: null, media: null, timestamp: Date.now(), reactions: {} } },
        }],
    } as any);
}

beforeEach(() => {
    resetStores();
    vi.mocked(invoke).mockClear();
    activeChannelId.set(ROOM);
    // Clear the room's timeline from prior tests
    handleMatrixEvent({ type: 'TimelineCleared', data: ROOM } as any);
});

describe('MessageInput', () => {
    it('Enter sends message', async () => {
        const user = userEvent.setup();
        render(MessageInput);

        const textarea = screen.getByRole('textbox');
        await user.click(textarea);
        await user.type(textarea, 'hello');
        await user.keyboard('{Enter}');

        expect(invoke).toHaveBeenCalledWith('core_command', {
            command: {
                type: 'Matrix',
                data: {
                    type: 'SendMessage',
                    data: { room_id: ROOM, text: 'hello', html_body: null, attachment_path: null },
                },
            },
        });
    });

    it('empty input does not send', async () => {
        const user = userEvent.setup();
        render(MessageInput);

        const textarea = screen.getByRole('textbox');
        await user.click(textarea);
        await user.keyboard('{Enter}');

        expect(invoke).not.toHaveBeenCalled();
    });

    it('Shift+Enter does not send', async () => {
        const user = userEvent.setup();
        render(MessageInput);

        const textarea = screen.getByRole('textbox');
        await user.click(textarea);
        await user.type(textarea, 'hello');
        vi.mocked(invoke).mockClear();

        await user.keyboard('{Shift>}{Enter}{/Shift}');

        expect(invoke).not.toHaveBeenCalledWith('core_command', expect.anything());
    });

    it('compose lock blocks Enter-to-send', async () => {
        const user = userEvent.setup();
        render(MessageInput);

        // Enable compose lock
        const lockBtn = screen.getByLabelText('Lock send (compose mode)');
        await user.click(lockBtn);

        const textarea = screen.getByRole('textbox');
        await user.click(textarea);
        await user.type(textarea, 'hello');
        vi.mocked(invoke).mockClear();

        await user.keyboard('{Enter}');

        // Enter should NOT trigger send in compose mode
        expect(invoke).not.toHaveBeenCalledWith('core_command', expect.anything());
    });

    it('compose lock shows send button that works', async () => {
        const user = userEvent.setup();
        render(MessageInput);

        // Enable compose lock
        const lockBtn = screen.getByLabelText('Lock send (compose mode)');
        await user.click(lockBtn);

        const textarea = screen.getByRole('textbox');
        await user.click(textarea);
        await user.type(textarea, 'hello');
        vi.mocked(invoke).mockClear();

        // Click the explicit send button (appears only in compose mode)
        const sendBtn = screen.getByLabelText('Send message');
        await user.click(sendBtn);

        expect(invoke).toHaveBeenCalledWith('core_command', expect.objectContaining({
            command: expect.objectContaining({ type: 'Matrix' }),
        }));
    });

    it('reply preview renders when replyingTo is set', async () => {
        const msg: ChatMessage = {
            id: '$reply1',
            sender: '@alice:etch.gg',
            body: 'original message',
            html_body: null,
            media: null,
            timestamp: Date.now(),
            reactions: {},
        };
        setReply(msg);
        render(MessageInput);

        // The sender is displayed as the part before ':'
        expect(screen.getByText('@alice')).toBeInTheDocument();
        expect(screen.getByText('original message')).toBeInTheDocument();
    });

    it('cancel reply clears reply state', async () => {
        const user = userEvent.setup();
        const msg: ChatMessage = {
            id: '$reply1',
            sender: '@alice:etch.gg',
            body: 'original message',
            html_body: null,
            media: null,
            timestamp: Date.now(),
            reactions: {},
        };
        setReply(msg);
        render(MessageInput);

        const cancelBtn = screen.getByLabelText('Cancel reply');
        await user.click(cancelBtn);

        expect(screen.queryByText('original message')).not.toBeInTheDocument();
    });

    it('emoji picker opens on button click', async () => {
        const user = userEvent.setup();
        const { container } = render(MessageInput);

        const emojiBtn = screen.getByLabelText('Emoji');
        await user.click(emojiBtn);

        expect(container.querySelector('.emoji-picker')).toBeInTheDocument();
    });

    it('emoji picker closes on outside click', async () => {
        const user = userEvent.setup();
        const { container } = render(MessageInput);

        // Open picker
        const emojiBtn = screen.getByLabelText('Emoji');
        await user.click(emojiBtn);
        expect(container.querySelector('.emoji-picker')).toBeInTheDocument();

        // Click outside (the textarea)
        const textarea = screen.getByRole('textbox');
        await user.click(textarea);

        expect(container.querySelector('.emoji-picker')).not.toBeInTheDocument();
    });

    it('emoji inserts into textarea', async () => {
        const user = userEvent.setup();
        const { container } = render(MessageInput);

        const emojiBtn = screen.getByLabelText('Emoji');
        await user.click(emojiBtn);

        // Click the first emoji in the grid
        const firstEmoji = container.querySelector('.emoji-cell')!;
        await user.click(firstEmoji);

        const textarea = screen.getByRole('textbox') as HTMLTextAreaElement;
        expect(textarea.value).toContain('😀');
    });

    it('tab completion inserts first match', async () => {
        seedUser(ROOM, '@alice:etch.gg', 'Alice');
        seedUser(ROOM, '@bob:etch.gg', 'Bob');

        render(MessageInput);
        const textarea = screen.getByRole('textbox') as HTMLTextAreaElement;

        // Type @al and press Tab
        await fireEvent.input(textarea, { target: { value: '@al' } });
        textarea.selectionStart = 3;
        textarea.selectionEnd = 3;
        await fireEvent.keyDown(textarea, { key: 'Tab' });

        expect(textarea.value).toBe('@Alice ');
    });

    it('tab completion cycles through matches', async () => {
        seedUser(ROOM, '@alice:etch.gg', 'Alice');
        seedUser(ROOM, '@alex:etch.gg', 'Alex');

        render(MessageInput);
        const textarea = screen.getByRole('textbox') as HTMLTextAreaElement;

        // Type @al and press Tab twice
        await fireEvent.input(textarea, { target: { value: '@al' } });
        textarea.selectionStart = 3;
        textarea.selectionEnd = 3;
        await fireEvent.keyDown(textarea, { key: 'Tab' });

        const firstMatch = textarea.value;
        await fireEvent.keyDown(textarea, { key: 'Tab' });

        // Should have cycled to the other match
        expect(textarea.value).not.toBe(firstMatch);
        expect(textarea.value).toMatch(/^@(Alice|Alex) $/);
    });

    it('non-Tab key resets tab cycling', async () => {
        seedUser(ROOM, '@alice:etch.gg', 'Alice');
        seedUser(ROOM, '@alex:etch.gg', 'Alex');

        render(MessageInput);
        const textarea = screen.getByRole('textbox') as HTMLTextAreaElement;

        // Type @al, Tab (first match), then type a character
        await fireEvent.input(textarea, { target: { value: '@al' } });
        textarea.selectionStart = 3;
        textarea.selectionEnd = 3;
        await fireEvent.keyDown(textarea, { key: 'Tab' });

        const firstMatch = textarea.value;

        // Type something (resets cycling)
        await fireEvent.keyDown(textarea, { key: 'a' });

        // Tab again should start fresh (not cycle to second match)
        await fireEvent.input(textarea, { target: { value: firstMatch + 'x' } });
        textarea.selectionStart = textarea.value.length;
        textarea.selectionEnd = textarea.value.length;
        await fireEvent.keyDown(textarea, { key: 'Tab' });

        // No @-word at cursor anymore, so tab completion does nothing
        expect(textarea.value).toBe(firstMatch + 'x');
    });
});
