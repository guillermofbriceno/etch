import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { stat, remove } from '@tauri-apps/plugin-fs';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWebview, type DragDropEvent } from '@tauri-apps/api/webview';
import { activeChannelId, setEditing, clearEditing, toastError, activeOverlay, uploadLimits } from '$lib/stores';
import { resetStores } from '$lib/stores/__tests__/helpers';
import { probeMediaInfo, type Inspection, type Verdict } from '$lib/attachments';
import MessageInput from '../MessageInput.svelte';

vi.mock('$lib/markdown', () => ({
    composeHtml: vi.fn((md: string) => `<p>${md}</p>\n`),
    insertMentionLinks: vi.fn((html: string) => html),
}));

vi.mock('$lib/attachments', async (importOriginal) => ({
    ...await importOriginal<typeof import('$lib/attachments')>(),
    probeMediaInfo: vi.fn(),
}));

const ROOM = 'room1';
const MIB = 1024 * 1024;
const LIMITS = { image_bytes: 5 * MIB, other_bytes: 2 * MIB };

let fileSizes: Record<string, number>;
let commands: Record<string, (args: Record<string, unknown>) => unknown>;

function inspection(mimetype: string, verdict: Verdict, limit = 5 * MIB): Inspection {
    return { mimetype, limit, verdict };
}

beforeEach(() => {
    resetStores();
    activeChannelId.set(ROOM);
    fileSizes = {};
    commands = {
        paste_clipboard_image: () => null,
        inspect_attachment: () => inspection('application/octet-stream', { type: 'Accept', compress_offered: false }),
    };
    vi.mocked(probeMediaInfo).mockReset();
    vi.mocked(probeMediaInfo).mockResolvedValue(null);

    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockImplementation(async (cmd: string, args?: unknown) => {
        const handler = commands[cmd];
        return handler ? handler(args as Record<string, unknown>) : undefined;
    });
    vi.mocked(stat).mockReset();
    vi.mocked(stat).mockImplementation(async (path) => {
        const size = fileSizes[String(path)];
        if (size === undefined) throw new Error('no such file');
        return { size } as Awaited<ReturnType<typeof stat>>;
    });
    vi.mocked(open).mockReset();
    vi.mocked(remove).mockClear();
    vi.mocked(getCurrentWebview().onDragDropEvent).mockClear();
});

function textarea(): HTMLTextAreaElement {
    return screen.getByRole('textbox') as HTMLTextAreaElement;
}

async function pick(path: string, size: number) {
    fileSizes[path] = size;
    vi.mocked(open).mockResolvedValueOnce(path);
    await fireEvent.click(screen.getByLabelText('Attach file'));
}

function clipboardFile(name: string, size: number): File {
    const bytes = new Uint8Array([1, 2, 3]);
    return { name, size, type: '', arrayBuffer: async () => bytes.buffer } as unknown as File;
}

async function pasteFiles(files: File[]) {
    await fireEvent.paste(textarea(), { clipboardData: { files } });
}

async function drop(...paths: string[]) {
    const calls = vi.mocked(getCurrentWebview().onDragDropEvent).mock.calls;
    const handler = calls[calls.length - 1][0];
    const payload = { type: 'drop', paths, position: { x: 0, y: 0 } } as DragDropEvent;
    handler({ event: 'tauri://drag-drop', id: 1, payload });
    await tick();
}

async function send() {
    await fireEvent.keyDown(textarea(), { key: 'Enter' });
}

function invocations(command: string) {
    return vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === command);
}

function discarded() {
    return invocations('discard_temp_upload').map(([, args]) => (args as { path: string }).path);
}

function sentCommands() {
    return vi.mocked(invoke).mock.calls
        .filter(([cmd]) => cmd === 'core_command')
        .map(([, args]) => (args as { command: { data: { type: string; data: Record<string, unknown> } } }).command.data);
}

async function expectToast(message: string) {
    await vi.waitFor(() => expect(get(toastError)).toBe(message));
}

function compressCheckbox(container: HTMLElement): HTMLInputElement | null {
    return container.querySelector('.compress-option input[type="checkbox"]');
}

describe('attaching', () => {
    it("rejects a file core rejects, with core's reason, and attaches nothing", async () => {
        uploadLimits.set(LIMITS);
        commands.inspect_attachment = () => inspection('video/mp4', {
            type: 'Reject', reason: 'it is 3 MB and the limit for this kind of file is 2 MB',
        }, 2 * MIB);
        render(MessageInput);

        await pick('/home/user/clip.mp4', 3 * MIB);

        await expectToast("Couldn't attach clip.mp4: it is 3 MB and the limit for this kind of file is 2 MB");
        expect(invoke).toHaveBeenCalledWith('inspect_attachment', { name: 'clip.mp4', size: 3 * MIB, limits: LIMITS });
        expect(screen.queryByText('clip.mp4')).not.toBeInTheDocument();
        await send();
        expect(sentCommands()).toEqual([]);
    });

    it('rejects a pasted file core rejects without writing it anywhere', async () => {
        commands.inspect_attachment = () => inspection('video/mp4', {
            type: 'Reject', reason: 'it is 3 MB and the limit for this kind of file is 2 MB',
        }, 2 * MIB);
        render(MessageInput);

        await pasteFiles([clipboardFile('clip.mp4', 3 * MIB)]);

        await expectToast("Couldn't attach clip.mp4: it is 3 MB and the limit for this kind of file is 2 MB");
        expect(invocations('save_pasted_file')).toEqual([]);
        expect(screen.queryByText('clip.mp4')).not.toBeInTheDocument();
    });

    it('keeps the newer file when an older attach finishes late', async () => {
        let finishOlder!: (meta: Awaited<ReturnType<typeof stat>>) => void;
        vi.mocked(stat).mockImplementationOnce(() => new Promise((resolve) => { finishOlder = resolve; }));
        fileSizes['/home/user/newer.pdf'] = 1000;
        render(MessageInput);
        vi.mocked(open).mockResolvedValueOnce('/home/user/older.pdf');
        await fireEvent.click(screen.getByLabelText('Attach file'));
        await vi.waitFor(() => expect(stat).toHaveBeenCalledWith('/home/user/older.pdf'));

        await drop('/home/user/newer.pdf');
        await vi.waitFor(() => expect(screen.getByText('newer.pdf')).toBeInTheDocument());
        finishOlder({ size: 1000 } as Awaited<ReturnType<typeof stat>>);
        await new Promise(resolve => setTimeout(resolve, 0));

        expect(screen.getByText('newer.pdf')).toBeInTheDocument();
        expect(screen.queryByText('older.pdf')).not.toBeInTheDocument();
    });
});

describe('sending an attachment', () => {
    it('forces compression on an image core says must be compressed', async () => {
        commands.inspect_attachment = () => inspection('image/png', { type: 'MustCompress' });
        const { container } = render(MessageInput);

        await pick('/home/user/photo.png', 9 * MIB);

        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());
        const checkbox = compressCheckbox(container)!;
        expect(checkbox.checked).toBe(true);
        expect(checkbox.disabled).toBe(true);
        expect(screen.getByText('Must be compressed to fit the 5 MB limit')).toBeInTheDocument();

        await send();

        await vi.waitFor(() => expect(sentCommands()).toHaveLength(1));
        expect(sentCommands()[0]).toEqual({
            type: 'SendAttachment',
            data: { room_id: ROOM, path: '/home/user/photo.png', compress: true, media_info: null },
        });
    });

    it('sends the image uncompressed when the user turns the offered compression off', async () => {
        commands.inspect_attachment = () => inspection('image/png', { type: 'Accept', compress_offered: true });
        const { container } = render(MessageInput);
        await pick('/home/user/photo.png', 400_000);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());
        const checkbox = compressCheckbox(container)!;
        expect(checkbox.disabled).toBe(false);

        await fireEvent.click(checkbox);
        await send();

        await vi.waitFor(() => expect(sentCommands()).toHaveLength(1));
        expect(sentCommands()[0].data).toMatchObject({ path: '/home/user/photo.png', compress: false });
    });

    it('probes a video as the type core named and sends what it measured', async () => {
        commands.inspect_attachment = () => inspection('video/mp4', { type: 'Accept', compress_offered: false }, 2 * MIB);
        vi.mocked(probeMediaInfo).mockResolvedValue({ width: 1280, height: 720, duration_ms: 4000 });
        const { container } = render(MessageInput);
        await pick('/home/user/clip.mp4', MIB);
        await vi.waitFor(() => expect(screen.getByText('clip.mp4')).toBeInTheDocument());
        expect(compressCheckbox(container)).toBeNull();

        await send();

        await vi.waitFor(() => expect(sentCommands()).toHaveLength(1));
        expect(probeMediaInfo).toHaveBeenCalledWith('/home/user/clip.mp4', MIB, 'video/mp4');
        expect(sentCommands()[0]).toEqual({
            type: 'SendAttachment',
            data: {
                room_id: ROOM,
                path: '/home/user/clip.mp4',
                compress: true,
                media_info: { width: 1280, height: 720, duration_ms: 4000 },
            },
        });
    });
});

describe('pasting', () => {
    it('has the shell write a pasted file, under its own name, and attaches the result', async () => {
        commands.save_pasted_file = () => '/tmp/etch-upload-1/résumé.pdf';
        render(MessageInput);

        await pasteFiles([clipboardFile('résumé.pdf', 3)]);

        await vi.waitFor(() => expect(screen.getByText('résumé.pdf')).toBeInTheDocument());
        expect(invocations('save_pasted_file')).toEqual([
            ['save_pasted_file', new Uint8Array([1, 2, 3]), { headers: { 'file-name': 'r%C3%A9sum%C3%A9.pdf' } }],
        ]);

        await send();

        await vi.waitFor(() => expect(sentCommands()).toHaveLength(1));
        expect(sentCommands()[0].data.path).toBe('/tmp/etch-upload-1/résumé.pdf');
    });

    it('prefers a clipboard bitmap over the file list', async () => {
        commands.paste_clipboard_image = () => ['/tmp/etch-paste-1.png', 1000];
        render(MessageInput);

        await pasteFiles([clipboardFile('image.png', 1000)]);

        await vi.waitFor(() => expect(screen.getByText('etch-paste-1.png')).toBeInTheDocument());
        expect(invocations('save_pasted_file')).toEqual([]);
    });

    it('asks the shell to discard a pasted file when the attachment is cleared', async () => {
        commands.paste_clipboard_image = () => ['/tmp/etch-paste-1/image.png', 1000];
        render(MessageInput);
        await pasteFiles([]);
        await vi.waitFor(() => expect(screen.getByText('image.png')).toBeInTheDocument());

        await fireEvent.click(screen.getByLabelText('Remove attachment'));

        expect(screen.queryByText('image.png')).not.toBeInTheDocument();
        expect(discarded()).toEqual(['/tmp/etch-paste-1/image.png']);
    });

    it('never deletes a file itself, even one picked from a folder named like a temp one', async () => {
        commands.paste_clipboard_image = () => ['/tmp/etch-upload-1/image.png', 1000];
        render(MessageInput);
        await pick('/home/user/etch-paste-holiday/photo.png', 1000);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());
        await pasteFiles([]);
        await vi.waitFor(() => expect(screen.getByText('image.png')).toBeInTheDocument());

        await fireEvent.click(screen.getByLabelText('Remove attachment'));

        expect(remove).not.toHaveBeenCalled();
    });

    it('shows a toast when the clipboard image cannot be read', async () => {
        commands.paste_clipboard_image = () => { throw 'Failed to create image from clipboard data'; };
        render(MessageInput);

        await pasteFiles([]);

        await expectToast("Couldn't paste the image: Failed to create image from clipboard data");
    });

    it('stays quiet when the clipboard holds no image', async () => {
        render(MessageInput);

        await fireEvent.paste(textarea(), { clipboardData: { files: [], types: ['text/plain'] } });
        await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('paste_clipboard_image'));
        await new Promise(resolve => setTimeout(resolve, 0));

        expect(get(toastError)).toBeNull();
    });
});

describe('a send that fails', () => {
    const FAILURE = 'core is shutting down';

    async function type(text: string) {
        await fireEvent.input(textarea(), { target: { value: text } });
    }

    it('restores the unsent text and attachment and says why', async () => {
        commands.core_command = () => { throw FAILURE; };
        render(MessageInput);
        await pick('/home/user/notes.pdf', 1000);
        await vi.waitFor(() => expect(screen.getByText('notes.pdf')).toBeInTheDocument());
        await type('hello');

        await send();

        await expectToast(`Couldn't send your message: ${FAILURE}`);
        expect(textarea().value).toBe('hello');
        expect(screen.getByText('notes.pdf')).toBeInTheDocument();
    });

    it('does not restore text that was already sent', async () => {
        commands.core_command = (args) => {
            if ((args as { command: { data: { type: string } } }).command.data.type === 'SendAttachment') throw FAILURE;
        };
        render(MessageInput);
        await pick('/home/user/notes.pdf', 1000);
        await vi.waitFor(() => expect(screen.getByText('notes.pdf')).toBeInTheDocument());
        await type('hello');

        await send();

        await expectToast(`Couldn't send notes.pdf: ${FAILURE}`);
        expect(textarea().value).toBe('');
        expect(screen.getByText('notes.pdf')).toBeInTheDocument();
    });

    it('keeps a draft the user started while the send was in flight', async () => {
        commands.paste_clipboard_image = () => ['/tmp/etch-paste-1/image.png', 1000];
        render(MessageInput);
        await pasteFiles([]);
        await vi.waitFor(() => expect(screen.getByText('image.png')).toBeInTheDocument());
        let fail!: (reason: unknown) => void;
        commands.core_command = () => new Promise((_, reject) => { fail = reject; });
        await type('hello');

        await send();
        await vi.waitFor(() => expect(sentCommands()).toHaveLength(1));
        await type('new draft');
        await pick('/home/user/notes.pdf', 1000);
        await vi.waitFor(() => expect(screen.getByText('notes.pdf')).toBeInTheDocument());
        fail(FAILURE);

        await expectToast(`Couldn't send your message: ${FAILURE}`);
        expect(textarea().value).toBe('new draft');
        expect(screen.getByText('notes.pdf')).toBeInTheDocument();
        expect(screen.queryByText('image.png')).not.toBeInTheDocument();
        expect(discarded()).toContain('/tmp/etch-paste-1/image.png');
    });
});

describe('drag and drop', () => {
    it('attaches the first dropped file', async () => {
        fileSizes['/home/user/photo.png'] = 1000;
        fileSizes['/home/user/other.png'] = 1000;
        render(MessageInput);

        await drop('/home/user/photo.png', '/home/user/other.png');

        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());
        expect(screen.queryByText('other.png')).not.toBeInTheDocument();
    });

    it('ignores a drop the input is in no state to take', async () => {
        fileSizes['/home/user/photo.png'] = 1000;
        render(MessageInput);
        const editing = {
            id: '$edit', sender: '@someone:example.org', body: 'draft', html_body: null,
            media: null, timestamp: 0, edited: false, reactions: {},
        };
        const blockers: [string, () => void, () => void][] = [
            ['an overlay covers it', () => activeOverlay.set('settings'), () => activeOverlay.set('none')],
            ['no channel is open', () => activeChannelId.set(null), () => activeChannelId.set(ROOM)],
            ['a message is being edited', () => setEditing(editing), () => clearEditing()],
        ];

        for (const [label, block, unblock] of blockers) {
            block();
            await tick();
            await drop('/home/user/photo.png');
            expect(stat, label).not.toHaveBeenCalled();
            unblock();
            await tick();
        }
    });
});
