import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { stat, readFile, remove } from '@tauri-apps/plugin-fs';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWebview, type DragDropEvent } from '@tauri-apps/api/webview';
import { activeChannelId, setEditing, clearEditing, toastError, activeOverlay, uploadLimits } from '$lib/stores';
import { resetStores } from '$lib/stores/__tests__/helpers';
import MessageInput from '../MessageInput.svelte';

vi.mock('$lib/markdown', () => ({
    composeHtml: vi.fn((md: string) => `<p>${md}</p>\n`),
    insertMentionLinks: vi.fn((html: string) => html),
}));

const ROOM = 'room1';
const MIB = 1024 * 1024;
const LIMITS = { image_bytes: 5 * MIB, other_bytes: 2 * MIB };

let fileSizes: Record<string, number>;
let commands: Record<string, (args: Record<string, unknown>) => unknown>;

beforeEach(() => {
    resetStores();
    activeChannelId.set(ROOM);
    fileSizes = {};
    commands = { paste_clipboard_image: () => null };

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
    vi.mocked(readFile).mockReset();
    vi.mocked(readFile).mockRejectedValue(new Error('no such file'));
    vi.mocked(open).mockReset();
    vi.mocked(remove).mockClear();
    vi.mocked(getCurrentWebview().onDragDropEvent).mockClear();
});

afterEach(() => {
    vi.unstubAllGlobals();
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

function sentMessages() {
    return vi.mocked(invoke).mock.calls
        .filter(([cmd]) => cmd === 'core_command')
        .map(([, args]) => (args as { command: { data: { data: Record<string, unknown> } } }).command.data.data);
}

async function expectToast(message: string) {
    await vi.waitFor(() => expect(get(toastError)).toBe(message));
}

function compressCheckbox(container: HTMLElement): HTMLInputElement | null {
    return container.querySelector('.compress-option input[type="checkbox"]');
}

describe('limits at attach time', () => {
    beforeEach(() => {
        uploadLimits.set(LIMITS);
    });

    it('rejects an other file over its limit, naming the limit', async () => {
        render(MessageInput);

        await pick('/home/user/clip.mp4', 3 * MIB);

        await expectToast("Couldn't attach clip.mp4: it is 3 MB and the limit for this kind of file is 2 MB");
        expect(screen.queryByText('clip.mp4')).not.toBeInTheDocument();
        await send();
        expect(sentMessages()).toEqual([]);
    });

    it('rejects a pasted file over its limit without writing it anywhere', async () => {
        render(MessageInput);

        await pasteFiles([clipboardFile('clip.mp4', 3 * MIB)]);

        await expectToast("Couldn't attach clip.mp4: it is 3 MB and the limit for this kind of file is 2 MB");
        expect(invocations('save_pasted_file')).toEqual([]);
        expect(screen.queryByText('clip.mp4')).not.toBeInTheDocument();
    });
});

describe('an image over its limit', () => {
    beforeEach(() => {
        uploadLimits.set(LIMITS);
    });

    it('stays attached, must be compressed, and is sent in its compressed form', async () => {
        commands.compress_image = () => '/tmp/etch-paste-c/photo.jpg';
        fileSizes['/tmp/etch-paste-c/photo.jpg'] = 2 * MIB;
        const { container } = render(MessageInput);

        await pick('/home/user/photo.png', 9 * MIB);

        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());
        const checkbox = compressCheckbox(container)!;
        expect(checkbox.checked).toBe(true);
        expect(checkbox.disabled).toBe(true);
        expect(screen.getByText('Must be compressed to fit the 5 MB limit')).toBeInTheDocument();

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(invoke).toHaveBeenCalledWith('compress_image', { path: '/home/user/photo.png' });
        expect(sentMessages()[0].attachment_path).toBe('/tmp/etch-paste-c/photo.jpg');
        expect(get(toastError)).toBeNull();
    });

    it('is not sent when compression leaves it over the limit', async () => {
        commands.compress_image = () => '/tmp/etch-paste-c/photo.jpg';
        fileSizes['/tmp/etch-paste-c/photo.jpg'] = 5.5 * MIB;
        render(MessageInput);
        await pick('/home/user/photo.png', 9 * MIB);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await expectToast("Couldn't send photo.png: it is 5.5 MB after compression and the limit for this kind of file is 5 MB");
        expect(sentMessages()).toEqual([]);
        expect(discarded()).toContain('/tmp/etch-paste-c/photo.jpg');
    });

    it.each<[string, (args: Record<string, unknown>) => unknown]>([
        ['hands it back unchanged', (args) => args.path],
        ['fails', () => { throw new Error('io error'); }],
    ])('is not sent in its original form when compression %s', async (_, compress) => {
        commands.compress_image = compress;
        render(MessageInput);
        await pick('/home/user/photo.png', 9 * MIB);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await expectToast("Couldn't send photo.png: it could not be compressed to fit the 5 MB limit");
        expect(sentMessages()).toEqual([]);
    });
});

describe('an image within its limit', () => {
    beforeEach(() => {
        uploadLimits.set(LIMITS);
    });

    it('falls back to the original when compression fails', async () => {
        commands.compress_image = () => { throw new Error('io error'); };
        const { container } = render(MessageInput);
        await pick('/home/user/photo.png', 400_000);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());
        expect(compressCheckbox(container)!.disabled).toBe(false);

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(invoke).toHaveBeenCalledWith('compress_image', { path: '/home/user/photo.png' });
        expect(sentMessages()[0].attachment_path).toBe('/home/user/photo.png');
        expect(get(toastError)).toBeNull();
    });
});

describe('GIFs', () => {
    it('are never sent to compress_image, however large', async () => {
        const { container } = render(MessageInput);
        await pick('/home/user/party.gif', 4 * MIB);
        await vi.waitFor(() => expect(screen.getByText('party.gif')).toBeInTheDocument());
        expect(compressCheckbox(container)).toBeNull();

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(invoke).not.toHaveBeenCalledWith('compress_image', expect.anything());
        expect(sentMessages()[0].attachment_path).toBe('/home/user/party.gif');
    });
});

describe('attaching', () => {
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

describe('media_info', () => {
    it('is probed from the compressed file that is actually sent, at its new size', async () => {
        commands.compress_image = () => '/tmp/etch-paste-c/photo.jpg';
        fileSizes['/tmp/etch-paste-c/photo.jpg'] = MIB;
        vi.mocked(readFile).mockResolvedValue(new Uint8Array([1, 2, 3]));
        vi.stubGlobal('createImageBitmap', vi.fn().mockResolvedValue({ width: 2048, height: 1536, close: vi.fn() }));
        render(MessageInput);
        await pick('/home/user/photo.png', 9 * MIB);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        // The 9 MB original is too large to probe, so a probe means the new size was used.
        expect(readFile).toHaveBeenCalledWith('/tmp/etch-paste-c/photo.jpg');
        expect(readFile).not.toHaveBeenCalledWith('/home/user/photo.png');
        expect(sentMessages()[0]).toMatchObject({
            attachment_path: '/tmp/etch-paste-c/photo.jpg',
            media_info: { width: 2048, height: 1536, duration_ms: null },
        });
    });

    it('is null when the file cannot be probed, and the file is still sent', async () => {
        render(MessageInput);
        await pick('/home/user/photo.png', 100_000);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(sentMessages()[0]).toMatchObject({ attachment_path: '/home/user/photo.png', media_info: null });
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

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(sentMessages()[0].attachment_path).toBe('/tmp/etch-upload-1/résumé.pdf');
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
            const data = (args as { command: { data: { data: { attachment_path: string | null } } } }).command.data.data;
            if (data.attachment_path) throw FAILURE;
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
        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
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

    it('restores the compressed output of a paste, since compression removed the original', async () => {
        commands.paste_clipboard_image = () => ['/tmp/etch-paste-1/image.png', 400_000];
        commands.compress_image = () => '/tmp/etch-paste-2/image.jpg';
        fileSizes['/tmp/etch-paste-2/image.jpg'] = 120_000;
        commands.core_command = () => { throw FAILURE; };
        render(MessageInput);
        await pasteFiles([]);
        await vi.waitFor(() => expect(screen.getByText('image.png')).toBeInTheDocument());

        await send();

        await expectToast(`Couldn't send image.png: ${FAILURE}`);
        expect(screen.getByText('image.jpg')).toBeInTheDocument();
        expect(screen.getByText('117.2 KB')).toBeInTheDocument();
        expect(screen.queryByText('image.png')).not.toBeInTheDocument();
    });

    it('restores a picked file and discards its compressed copy', async () => {
        commands.compress_image = () => '/tmp/etch-paste-c/photo.jpg';
        fileSizes['/tmp/etch-paste-c/photo.jpg'] = 100_000;
        commands.core_command = () => { throw FAILURE; };
        render(MessageInput);
        await pick('/home/user/photo.png', 400_000);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await expectToast(`Couldn't send photo.png: ${FAILURE}`);
        expect(screen.getByText('photo.png')).toBeInTheDocument();
        expect(discarded()).toContain('/tmp/etch-paste-c/photo.jpg');
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
