import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { stat, readFile, writeFile, mkdir, remove } from '@tauri-apps/plugin-fs';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWebview, type DragDropEvent } from '@tauri-apps/api/webview';
import { activeChannelId, setEditing, toastError, activeOverlay, uploadLimits } from '$lib/stores';
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
    vi.mocked(writeFile).mockClear();
    vi.mocked(mkdir).mockClear();
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

function dragDrop(payload: DragDropEvent) {
    const calls = vi.mocked(getCurrentWebview().onDragDropEvent).mock.calls;
    const handler = calls[calls.length - 1][0];
    handler({ event: 'tauri://drag-drop', id: 1, payload });
}

async function drop(...paths: string[]) {
    dragDrop({ type: 'drop', paths, position: { x: 0, y: 0 } } as DragDropEvent);
    await tick();
}

async function send() {
    await fireEvent.keyDown(textarea(), { key: 'Enter' });
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

    it('holds a GIF to the other limit', async () => {
        render(MessageInput);

        await pick('/home/user/party.gif', 3 * MIB);

        await expectToast("Couldn't attach party.gif: it is 3 MB and the limit for this kind of file is 2 MB");
        expect(screen.queryByText('party.gif')).not.toBeInTheDocument();
    });

    it('rejects a dropped file over its limit', async () => {
        fileSizes['/home/user/clip.mp4'] = 3 * MIB;
        render(MessageInput);

        await drop('/home/user/clip.mp4');

        await expectToast("Couldn't attach clip.mp4: it is 3 MB and the limit for this kind of file is 2 MB");
        expect(screen.queryByText('clip.mp4')).not.toBeInTheDocument();
    });

    it('rejects a pasted file over its limit without writing it anywhere', async () => {
        render(MessageInput);

        await pasteFiles([clipboardFile('clip.mp4', 3 * MIB)]);

        await expectToast("Couldn't attach clip.mp4: it is 3 MB and the limit for this kind of file is 2 MB");
        expect(writeFile).not.toHaveBeenCalled();
        expect(mkdir).not.toHaveBeenCalled();
        expect(screen.queryByText('clip.mp4')).not.toBeInTheDocument();
    });

    it('removes a pasted bitmap that is over its limit and cannot be compressed', async () => {
        commands.paste_clipboard_image = () => ['/tmp/etch-paste-9/logo.svg', 6 * MIB];
        render(MessageInput);

        await pasteFiles([]);

        await expectToast("Couldn't attach logo.svg: it is 6 MB and the limit for this kind of file is 5 MB");
        expect(remove).toHaveBeenCalledWith('/tmp/etch-paste-9', { recursive: true });
    });

    it('leaves a file whose type only core can judge to core', async () => {
        render(MessageInput);

        await pick('/home/user/raw.dng', 3 * MIB);
        await vi.waitFor(() => expect(screen.getByText('raw.dng')).toBeInTheDocument());
        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(sentMessages()[0].attachment_path).toBe('/home/user/raw.dng');
        expect(get(toastError)).toBeNull();
    });

    it('refuses an oversize image the shell cannot compress', async () => {
        const { container } = render(MessageInput);

        await pick('/home/user/photo.heic', 7 * MIB);

        await expectToast("Couldn't attach photo.heic: it is 7 MB and the limit for this kind of file is 5 MB");
        expect(screen.queryByText('photo.heic')).not.toBeInTheDocument();
        expect(compressCheckbox(container)).toBeNull();
    });

    it('attaches an other file within its limit and shows its size', async () => {
        render(MessageInput);

        await pick('/home/user/notes.pdf', MIB);

        await vi.waitFor(() => expect(screen.getByText('notes.pdf')).toBeInTheDocument());
        expect(screen.getByText('1 MB')).toBeInTheDocument();
        expect(get(toastError)).toBeNull();
    });
});

describe('an image over its limit', () => {
    beforeEach(() => {
        uploadLimits.set(LIMITS);
    });

    it('stays attached but must be compressed', async () => {
        const { container } = render(MessageInput);

        await pick('/home/user/photo.png', 9 * MIB);

        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());
        const checkbox = compressCheckbox(container)!;
        expect(checkbox.checked).toBe(true);
        expect(checkbox.disabled).toBe(true);
        expect(screen.getByText('Must be compressed to fit the 5 MB limit')).toBeInTheDocument();
        expect(get(toastError)).toBeNull();
    });

    it('is compressed before it is sent', async () => {
        commands.compress_image = () => '/tmp/etch-paste-c/photo.jpg';
        fileSizes['/tmp/etch-paste-c/photo.jpg'] = 2 * MIB;
        render(MessageInput);
        await pick('/home/user/photo.png', 9 * MIB);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(invoke).toHaveBeenCalledWith('compress_image', { path: '/home/user/photo.png' });
        expect(sentMessages()[0].attachment_path).toBe('/tmp/etch-paste-c/photo.jpg');
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
        expect(remove).toHaveBeenCalledWith('/tmp/etch-paste-c', { recursive: true });
    });

    it('is not sent when compression cannot shrink it', async () => {
        commands.compress_image = (args) => args.path;
        render(MessageInput);
        await pick('/home/user/photo.png', 9 * MIB);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await expectToast("Couldn't send photo.png: it could not be compressed to fit the 5 MB limit");
        expect(sentMessages()).toEqual([]);
    });

    it('is not sent in its original form when compression fails', async () => {
        commands.compress_image = () => { throw new Error('io error'); };
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

describe('while the limits are unknown', () => {
    it('attaches and sends a file of any size without checking it', async () => {
        render(MessageInput);

        await pick('/home/user/clip.mp4', 50 * MIB);
        await vi.waitFor(() => expect(screen.getByText('clip.mp4')).toBeInTheDocument());
        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(sentMessages()[0].attachment_path).toBe('/home/user/clip.mp4');
        expect(get(toastError)).toBeNull();
    });

    it('leaves compressing a large image up to the user', async () => {
        const { container } = render(MessageInput);
        await pick('/home/user/photo.png', 9 * MIB);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        const checkbox = compressCheckbox(container)!;
        expect(checkbox.disabled).toBe(false);
        expect(screen.queryByText(/Must be compressed/)).not.toBeInTheDocument();
        await fireEvent.click(checkbox);
        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(invoke).not.toHaveBeenCalledWith('compress_image', expect.anything());
        expect(sentMessages()[0].attachment_path).toBe('/home/user/photo.png');
    });

    it('measures the compressed file and probes it at its new size', async () => {
        commands.compress_image = () => '/tmp/etch-paste-c/photo.jpg';
        fileSizes['/tmp/etch-paste-c/photo.jpg'] = MIB;
        vi.mocked(readFile).mockResolvedValue(new Uint8Array([1, 2, 3]));
        vi.stubGlobal('createImageBitmap', vi.fn().mockResolvedValue({ width: 2048, height: 1536, close: vi.fn() }));
        render(MessageInput);
        await pick('/home/user/photo.png', 9 * MIB);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(stat).toHaveBeenCalledWith('/tmp/etch-paste-c/photo.jpg');
        // The 9 MB original is too large to probe, so a probe means the new size was used.
        expect(readFile).toHaveBeenCalledWith('/tmp/etch-paste-c/photo.jpg');
        expect(sentMessages()[0]).toMatchObject({
            attachment_path: '/tmp/etch-paste-c/photo.jpg',
            media_info: { width: 2048, height: 1536, duration_ms: null },
        });
    });
});

describe('media_info', () => {
    it('carries the probed dimensions of an image in SendMessage', async () => {
        vi.mocked(readFile).mockResolvedValue(new Uint8Array([1, 2, 3]));
        vi.stubGlobal('createImageBitmap', vi.fn().mockResolvedValue({ width: 640, height: 480, close: vi.fn() }));
        render(MessageInput);
        await pick('/home/user/photo.png', 100_000);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(sentMessages()[0]).toEqual({
            room_id: ROOM,
            text: '',
            html_body: null,
            attachment_path: '/home/user/photo.png',
            media_info: { width: 640, height: 480, duration_ms: null },
        });
    });

    it('is probed from the compressed file that is actually sent', async () => {
        commands.compress_image = () => '/tmp/etch-paste-c/photo.jpg';
        vi.mocked(readFile).mockResolvedValue(new Uint8Array([1, 2, 3]));
        vi.stubGlobal('createImageBitmap', vi.fn().mockResolvedValue({ width: 2048, height: 1536, close: vi.fn() }));
        render(MessageInput);
        await pick('/home/user/photo.png', 400_000);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(readFile).toHaveBeenCalledWith('/tmp/etch-paste-c/photo.jpg');
        expect(readFile).not.toHaveBeenCalledWith('/home/user/photo.png');
        expect(sentMessages()[0].media_info).toEqual({ width: 2048, height: 1536, duration_ms: null });
    });

    it('is null when the file cannot be probed', async () => {
        render(MessageInput);
        await pick('/home/user/photo.png', 100_000);
        await vi.waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(sentMessages()[0].media_info).toBeNull();
    });

    it('is null for a plain file, which is never read', async () => {
        render(MessageInput);
        await pick('/home/user/notes.pdf', 100_000);
        await vi.waitFor(() => expect(screen.getByText('notes.pdf')).toBeInTheDocument());

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(sentMessages()[0].media_info).toBeNull();
        expect(readFile).not.toHaveBeenCalled();
    });
});

describe('pasting copied files', () => {
    it('writes the file into its own etch-paste directory and attaches it', async () => {
        render(MessageInput);

        await pasteFiles([clipboardFile('notes.pdf', 3)]);

        await vi.waitFor(() => expect(screen.getByText('notes.pdf')).toBeInTheDocument());
        const dir = vi.mocked(mkdir).mock.calls[0][0] as string;
        expect(dir).toMatch(/^\/tmp\/etch-paste-[^/]+$/);
        expect(mkdir).toHaveBeenCalledWith(dir, { recursive: true });
        expect(writeFile).toHaveBeenCalledWith(`${dir}/notes.pdf`, new Uint8Array([1, 2, 3]));

        await send();

        await vi.waitFor(() => expect(sentMessages()).toHaveLength(1));
        expect(sentMessages()[0].attachment_path).toBe(`${dir}/notes.pdf`);
    });

    it('prefers a clipboard bitmap over the file list', async () => {
        commands.paste_clipboard_image = () => ['/tmp/etch-paste-1.png', 1000];
        render(MessageInput);

        await pasteFiles([clipboardFile('image.png', 1000)]);

        await vi.waitFor(() => expect(screen.getByText('etch-paste-1.png')).toBeInTheDocument());
        expect(writeFile).not.toHaveBeenCalled();
    });

    it('removes a pasted temp file when the attachment is cleared', async () => {
        commands.paste_clipboard_image = () => ['/tmp/etch-paste-1/image.png', 1000];
        render(MessageInput);
        await pasteFiles([]);
        await vi.waitFor(() => expect(screen.getByText('image.png')).toBeInTheDocument());

        await fireEvent.click(screen.getByLabelText('Remove attachment'));

        expect(screen.queryByText('image.png')).not.toBeInTheDocument();
        expect(remove).toHaveBeenCalledWith('/tmp/etch-paste-1', { recursive: true });
    });

    it('leaves a flat etch-paste file in place, since only the directory form is temp', async () => {
        commands.paste_clipboard_image = () => ['/tmp/etch-paste-1.png', 1000];
        render(MessageInput);
        await pasteFiles([]);
        await vi.waitFor(() => expect(screen.getByText('etch-paste-1.png')).toBeInTheDocument());

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

    it('falls back to the file list when the clipboard image cannot be read', async () => {
        commands.paste_clipboard_image = () => { throw 'Failed to create image from clipboard data'; };
        render(MessageInput);

        await pasteFiles([clipboardFile('notes.pdf', 3)]);

        await vi.waitFor(() => expect(screen.getByText('notes.pdf')).toBeInTheDocument());
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
        expect(remove).toHaveBeenCalledWith('/tmp/etch-paste-1', { recursive: true });
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
        expect(remove).toHaveBeenCalledWith('/tmp/etch-paste-c', { recursive: true });
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

    it('ignores a drop while an overlay covers the input', async () => {
        fileSizes['/home/user/photo.png'] = 1000;
        render(MessageInput);
        activeOverlay.set('settings');
        await tick();

        await drop('/home/user/photo.png');

        expect(stat).not.toHaveBeenCalled();
        expect(screen.queryByText('photo.png')).not.toBeInTheDocument();
    });

    it('ignores a drop when no channel is open', async () => {
        fileSizes['/home/user/photo.png'] = 1000;
        render(MessageInput);
        activeChannelId.set(null);
        await tick();

        await drop('/home/user/photo.png');

        expect(stat).not.toHaveBeenCalled();
    });

    it('ignores a drop while a message is being edited', async () => {
        fileSizes['/home/user/photo.png'] = 1000;
        render(MessageInput);
        setEditing({
            id: '$edit', sender: '@someone:example.org', body: 'draft', html_body: null,
            media: null, timestamp: 0, edited: false, reactions: {},
        });
        await tick();

        await drop('/home/user/photo.png');

        expect(stat).not.toHaveBeenCalled();
    });

    it('highlights the input while files are dragged over it', async () => {
        const { container } = render(MessageInput);
        const wrapper = container.querySelector('.input-wrapper')!;

        dragDrop({ type: 'enter', paths: ['/home/user/photo.png'], position: { x: 0, y: 0 } } as DragDropEvent);
        await tick();
        expect(wrapper).toHaveClass('drop-target');

        dragDrop({ type: 'leave' });
        await tick();
        expect(wrapper).not.toHaveClass('drop-target');
    });

    it('stops listening once the input is gone', async () => {
        const { unmount } = render(MessageInput);
        const listening = vi.mocked(getCurrentWebview().onDragDropEvent).mock.results.at(-1)!.value as Promise<() => void>;
        const unlisten = vi.mocked(await listening);
        unlisten.mockClear();

        unmount();

        expect(unlisten).toHaveBeenCalledOnce();
    });
});
