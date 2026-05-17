import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { tick } from 'svelte';
import { invoke } from '@tauri-apps/api/core';
import { stat } from '@tauri-apps/plugin-fs';
import { open } from '@tauri-apps/plugin-dialog';
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

    describe('image compression', () => {
        it('shows compress checkbox for images over 256KB', async () => {
            // Simulate a paste returning a large image
            vi.mocked(invoke).mockResolvedValueOnce(['/tmp/etch-paste-123.png', 300_000]);

            const { container } = render(MessageInput);
            const textarea = screen.getByRole('textbox');

            await fireEvent.paste(textarea);
            // Wait for async paste handler
            await vi.waitFor(() => {
                expect(container.querySelector('.compress-option')).toBeInTheDocument();
            });
        });

        it('hides compress checkbox for images under 256KB', async () => {
            // Simulate a paste returning a small image
            vi.mocked(invoke).mockResolvedValueOnce(['/tmp/etch-paste-123.png', 100_000]);

            const { container } = render(MessageInput);
            const textarea = screen.getByRole('textbox');

            await fireEvent.paste(textarea);
            await vi.waitFor(() => {
                expect(screen.getByText('etch-paste-123.png')).toBeInTheDocument();
            });
            expect(container.querySelector('.compress-option')).not.toBeInTheDocument();
        });

        it('calls compress_image before sending when checkbox is checked', async () => {
            vi.mocked(invoke)
                .mockResolvedValueOnce(['/tmp/etch-paste-123.png', 300_000]) // paste_clipboard_image
                .mockResolvedValueOnce('/tmp/etch-paste-123.jpg') // compress_image
                .mockResolvedValue(undefined); // core_command (sendMessage)

            render(MessageInput);
            const textarea = screen.getByRole('textbox') as HTMLTextAreaElement;

            await fireEvent.paste(textarea);
            await vi.waitFor(() => {
                expect(screen.getByText('etch-paste-123.png')).toBeInTheDocument();
            });

            // Compress checkbox should be checked by default; press Enter to send
            await fireEvent.keyDown(textarea, { key: 'Enter' });

            await vi.waitFor(() => {
                expect(invoke).toHaveBeenCalledWith('compress_image', { path: '/tmp/etch-paste-123.png' });
            });
            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: {
                    type: 'Matrix',
                    data: {
                        type: 'SendMessage',
                        data: { room_id: ROOM, text: '', html_body: null, attachment_path: '/tmp/etch-paste-123.jpg' },
                    },
                },
            });
        });

        it('skips compress_image when checkbox is unchecked', async () => {
            vi.mocked(invoke)
                .mockResolvedValueOnce(['/tmp/etch-paste-123.png', 300_000]) // paste_clipboard_image
                .mockResolvedValue(undefined); // core_command (sendMessage)

            const { container } = render(MessageInput);
            const textarea = screen.getByRole('textbox') as HTMLTextAreaElement;

            await fireEvent.paste(textarea);
            await vi.waitFor(() => {
                expect(screen.getByText('etch-paste-123.png')).toBeInTheDocument();
            });

            // Uncheck the compress checkbox
            const checkbox = container.querySelector('.compress-option input[type="checkbox"]') as HTMLInputElement;
            await fireEvent.click(checkbox);
            await tick();

            await fireEvent.keyDown(textarea, { key: 'Enter' });

            await vi.waitFor(() => {
                expect(invoke).toHaveBeenCalledWith('core_command', expect.anything());
            });
            expect(invoke).not.toHaveBeenCalledWith('compress_image', expect.anything());
            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: {
                    type: 'Matrix',
                    data: {
                        type: 'SendMessage',
                        data: { room_id: ROOM, text: '', html_body: null, attachment_path: '/tmp/etch-paste-123.png' },
                    },
                },
            });
        });

        it('shows compress checkbox for large file picker images', async () => {
            vi.mocked(open).mockResolvedValueOnce('/home/user/photo.jpg');
            vi.mocked(stat).mockResolvedValueOnce({ size: 500_000, isFile: true, isDirectory: false } as any);

            const { container } = render(MessageInput);
            const attachBtn = screen.getByLabelText('Attach file');
            await fireEvent.click(attachBtn);

            await vi.waitFor(() => {
                expect(screen.getByText('photo.jpg')).toBeInTheDocument();
            });
            expect(container.querySelector('.compress-option')).toBeInTheDocument();
        });

        it('hides compress checkbox for non-image files', async () => {
            vi.mocked(invoke).mockResolvedValueOnce(['/tmp/etch-paste-123.txt', 500_000]);

            const { container } = render(MessageInput);
            const textarea = screen.getByRole('textbox');

            // Simulate picking a non-image file by mocking the dialog
            vi.mocked(open).mockResolvedValueOnce('/home/user/document.pdf');
            vi.mocked(stat).mockResolvedValueOnce({ size: 500_000, isFile: true, isDirectory: false } as any);

            const attachBtn = screen.getByLabelText('Attach file');
            await fireEvent.click(attachBtn);

            await vi.waitFor(() => {
                expect(screen.getByText('document.pdf')).toBeInTheDocument();
            });
            expect(container.querySelector('.compress-option')).not.toBeInTheDocument();
        });
    });
});
