import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { readFile, writeFile, mkdir, remove } from '@tauri-apps/plugin-fs';
import {
    categoryOf,
    checkAttachment,
    discardTempFile,
    fileName,
    isCompressible,
    isTempPath,
    limitFor,
    mimeFromPath,
    probeMediaInfo,
    writeTempAttachment,
} from '../attachments';

const MIB = 1024 * 1024;
const LIMITS = { image_bytes: 5 * MIB, other_bytes: 2 * MIB };

describe('the category rule', () => {
    it('puts every image type except GIF in the image category', () => {
        expect(categoryOf('image/png')).toBe('image');
        expect(categoryOf('image/jpeg')).toBe('image');
        expect(categoryOf('image/webp')).toBe('image');
        expect(categoryOf('image/svg+xml')).toBe('image');
    });

    it('puts GIFs, video, audio and files in the other category', () => {
        expect(categoryOf('image/gif')).toBe('other');
        expect(categoryOf('video/mp4')).toBe('other');
        expect(categoryOf('audio/mpeg')).toBe('other');
        expect(categoryOf('application/pdf')).toBe('other');
        expect(categoryOf(null)).toBe('other');
    });

    it('reads the type off the extension, ignoring case and directories', () => {
        expect(mimeFromPath('/home/user/Photo.JPG')).toBe('image/jpeg');
        expect(mimeFromPath('C:\\Users\\user\\clip.webm')).toBe('video/webm');
        expect(mimeFromPath('/home/user/archive.tar.gz')).toBe('application/gzip');
        expect(fileName('C:\\Users\\user\\clip.webm')).toBe('clip.webm');
    });

    it('treats a file with no extension as a plain file, as core does', () => {
        expect(mimeFromPath('/home/user/README')).toBe('application/octet-stream');
        expect(mimeFromPath('/home/user/.bashrc')).toBe('application/octet-stream');
        expect(limitFor('/home/user/README', LIMITS)).toBe(2 * MIB);
    });

    it('does not guess the type of an extension it does not know', () => {
        expect(mimeFromPath('/home/user/raw.dng')).toBeNull();
        expect(mimeFromPath('/home/user/scan.jp2')).toBeNull();
    });

    it('picks the limit for the category', () => {
        expect(limitFor('/tmp/a.png', LIMITS)).toBe(5 * MIB);
        expect(limitFor('/tmp/a.gif', LIMITS)).toBe(2 * MIB);
        expect(limitFor('/tmp/a.mp4', LIMITS)).toBe(2 * MIB);
        expect(limitFor('/tmp/a.pdf', LIMITS)).toBe(2 * MIB);
        expect(limitFor('/tmp/a.png', null)).toBeNull();
    });

    it('has no limit to offer for an unknown extension, leaving it to core', () => {
        expect(limitFor('/home/user/raw.dng', LIMITS)).toBeNull();
        expect(limitFor('/home/user/photo.nef', LIMITS)).toBeNull();
    });
});

describe('isCompressible', () => {
    it('allows exactly the formats the shell re-encodes', () => {
        for (const ext of ['png', 'jpg', 'jpeg', 'jpe', 'jfif', 'webp', 'bmp', 'dib', 'tif', 'tiff', 'PNG']) {
            expect(isCompressible(`/tmp/photo.${ext}`)).toBe(true);
        }
    });

    it('never offers animated or vector images', () => {
        expect(isCompressible('/tmp/party.gif')).toBe(false);
        expect(isCompressible('/tmp/spin.apng')).toBe(false);
        expect(isCompressible('/tmp/logo.svg')).toBe(false);
    });

    it('never offers images the shell hands back unchanged', () => {
        for (const ext of ['avif', 'heic', 'heif', 'jxl', 'ico', 'dng']) {
            expect(isCompressible(`/tmp/photo.${ext}`)).toBe(false);
        }
    });

    it('never offers anything that is not an image', () => {
        expect(isCompressible('/tmp/clip.mp4')).toBe(false);
        expect(isCompressible('/tmp/notes.pdf')).toBe(false);
    });
});

describe('checkAttachment', () => {
    it('accepts anything while the limits are unknown', () => {
        expect(checkAttachment('/tmp/huge.mp4', 900 * MIB, null)).toEqual({ accept: true, mustCompress: false });
        expect(checkAttachment('/tmp/huge.png', 900 * MIB, null)).toEqual({ accept: true, mustCompress: false });
    });

    it('accepts files within their limit', () => {
        expect(checkAttachment('/tmp/clip.mp4', 2 * MIB, LIMITS)).toEqual({ accept: true, mustCompress: false });
        expect(checkAttachment('/tmp/photo.png', 5 * MIB, LIMITS)).toEqual({ accept: true, mustCompress: false });
    });

    it('rejects an other file over its limit, naming the limit', () => {
        expect(checkAttachment('/tmp/clip.mp4', 3 * MIB, LIMITS)).toEqual({
            accept: false,
            reason: 'it is 3 MB and the limit for this kind of file is 2 MB',
        });
    });

    it('holds a GIF to the other limit', () => {
        expect(checkAttachment('/tmp/party.gif', 3 * MIB, LIMITS)).toEqual({
            accept: false,
            reason: 'it is 3 MB and the limit for this kind of file is 2 MB',
        });
    });

    it('accepts an oversize image but requires compression', () => {
        expect(checkAttachment('/tmp/photo.png', 9 * MIB, LIMITS)).toEqual({ accept: true, mustCompress: true });
    });

    it('rejects an oversize image that compression cannot shrink', () => {
        expect(checkAttachment('/tmp/logo.svg', 6 * MIB, LIMITS)).toEqual({
            accept: false,
            reason: 'it is 6 MB and the limit for this kind of file is 5 MB',
        });
        expect(checkAttachment('/tmp/photo.heic', 7 * MIB, LIMITS)).toEqual({
            accept: false,
            reason: 'it is 7 MB and the limit for this kind of file is 5 MB',
        });
    });

    it('leaves a file of unknown type for core to judge', () => {
        expect(checkAttachment('/home/user/raw.dng', 3 * MIB, LIMITS)).toEqual({ accept: true, mustCompress: false });
    });
});

describe('probeMediaInfo', () => {
    let createdElements: HTMLElement[];
    // Restored one by one: restoreAllMocks would also strip the shared Tauri mocks of their defaults.
    let spies: { mockRestore: () => void }[];

    beforeEach(() => {
        vi.mocked(readFile).mockReset();
        vi.mocked(readFile).mockResolvedValue(new Uint8Array([1, 2, 3]));
        Object.defineProperty(URL, 'createObjectURL', { value: vi.fn(() => 'blob:probe'), configurable: true, writable: true });
        Object.defineProperty(URL, 'revokeObjectURL', { value: vi.fn(), configurable: true, writable: true });
        createdElements = [];
        const create = document.createElement.bind(document);
        spies = [
            vi.spyOn(document, 'createElement').mockImplementation(((tag: string, options?: ElementCreationOptions) => {
                const element = create(tag, options);
                createdElements.push(element);
                return element;
            }) as typeof document.createElement),
        ];
    });

    afterEach(() => {
        vi.mocked(readFile).mockReset();
        vi.mocked(readFile).mockRejectedValue(new Error('no such file'));
        for (const spy of spies) spy.mockRestore();
        vi.unstubAllGlobals();
        vi.useRealTimers();
    });

    async function loadingElement(tag: 'VIDEO' | 'AUDIO'): Promise<HTMLMediaElement> {
        await vi.waitFor(() => {
            expect(createdElements.some(e => e.tagName === tag && e.getAttribute('src') === 'blob:probe')).toBe(true);
        });
        return createdElements.find(e => e.tagName === tag) as HTMLMediaElement;
    }

    it('reads image dimensions from the decoded bitmap', async () => {
        const close = vi.fn();
        const decode = vi.fn().mockResolvedValue({ width: 640, height: 480, close });
        vi.stubGlobal('createImageBitmap', decode);

        const info = await probeMediaInfo('/tmp/photo.png', 1000);

        expect(info).toEqual({ width: 640, height: 480, duration_ms: null });
        expect(readFile).toHaveBeenCalledWith('/tmp/photo.png');
        expect((decode.mock.calls[0][0] as Blob).type).toBe('image/png');
        expect(close).toHaveBeenCalled();
    });

    it('falls back to an image element when bitmaps are unavailable', async () => {
        vi.stubGlobal('createImageBitmap', undefined);
        const setSrc = vi.spyOn(HTMLImageElement.prototype, 'src', 'set').mockImplementation(function (this: HTMLImageElement) {
            queueMicrotask(() => this.dispatchEvent(new Event('load')));
        });
        spies.push(
            setSrc,
            vi.spyOn(HTMLImageElement.prototype, 'naturalWidth', 'get').mockReturnValue(320),
            vi.spyOn(HTMLImageElement.prototype, 'naturalHeight', 'get').mockReturnValue(200),
        );

        const info = await probeMediaInfo('/tmp/photo.jpg', 1000);

        expect(info).toEqual({ width: 320, height: 200, duration_ms: null });
        expect(setSrc).toHaveBeenCalledWith('blob:probe');
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:probe');
    });

    it('reads video dimensions and duration from the loaded metadata', async () => {
        const pending = probeMediaInfo('/tmp/clip.mp4', 1000);
        const video = await loadingElement('VIDEO');
        Object.defineProperty(video, 'videoWidth', { value: 1920 });
        Object.defineProperty(video, 'videoHeight', { value: 1080 });
        Object.defineProperty(video, 'duration', { value: 12.3456 });
        video.dispatchEvent(new Event('loadedmetadata'));

        expect(await pending).toEqual({ width: 1920, height: 1080, duration_ms: 12346 });
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:probe');
        expect(video.hasAttribute('src')).toBe(false);
    });

    it('reads only the duration of audio', async () => {
        const pending = probeMediaInfo('/tmp/voice.ogg', 1000);
        const audio = await loadingElement('AUDIO');
        Object.defineProperty(audio, 'duration', { value: 3.5 });
        audio.dispatchEvent(new Event('loadedmetadata'));

        expect(await pending).toEqual({ width: null, height: null, duration_ms: 3500 });
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:probe');
    });

    it('gives null for a stream with no usable metadata', async () => {
        const pending = probeMediaInfo('/tmp/live.weba', 1000);
        const audio = await loadingElement('AUDIO');
        Object.defineProperty(audio, 'duration', { value: Infinity });
        audio.dispatchEvent(new Event('loadedmetadata'));

        expect(await pending).toBeNull();
    });

    it('gives null when the media cannot be decoded', async () => {
        const pending = probeMediaInfo('/tmp/clip.mkv', 1000);
        const video = await loadingElement('VIDEO');
        video.dispatchEvent(new Event('error'));

        expect(await pending).toBeNull();
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:probe');
    });

    it('gives up after a short timeout and still revokes the object url', async () => {
        vi.useFakeTimers();
        const pending = probeMediaInfo('/tmp/clip.mp4', 1000);

        await vi.advanceTimersByTimeAsync(3000);

        expect(await pending).toBeNull();
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:probe');
    });

    it('gives null when the file cannot be read', async () => {
        vi.mocked(readFile).mockRejectedValue(new Error('denied'));

        expect(await probeMediaInfo('/tmp/photo.png', 1000)).toBeNull();
    });

    it('does not read files that carry no media metadata', async () => {
        expect(await probeMediaInfo('/tmp/notes.pdf', 1000)).toBeNull();
        expect(readFile).not.toHaveBeenCalled();
    });

    it('does not read files too large to send', async () => {
        expect(await probeMediaInfo('/tmp/clip.mp4', 50 * MIB)).toBeNull();
        expect(readFile).not.toHaveBeenCalled();
    });
});

describe('writeTempAttachment', () => {
    beforeEach(() => {
        vi.mocked(mkdir).mockClear();
        vi.mocked(writeFile).mockReset();
        vi.mocked(remove).mockClear();
    });

    it('writes into a fresh etch-paste directory under the real file name', async () => {
        const bytes = new Uint8Array([1, 2, 3]);

        const path = await writeTempAttachment('notes.pdf', bytes);

        const dir = vi.mocked(mkdir).mock.calls[0][0] as string;
        expect(dir).toMatch(/^\/tmp\/etch-paste-[^/]+$/);
        expect(mkdir).toHaveBeenCalledWith(dir, { recursive: true });
        expect(path).toBe(`${dir}/notes.pdf`);
        expect(writeFile).toHaveBeenCalledWith(path, bytes);
    });

    it('gives every paste its own directory', async () => {
        const first = await writeTempAttachment('a.txt', new Uint8Array());
        const second = await writeTempAttachment('a.txt', new Uint8Array());

        expect(first).not.toBe(second);
    });

    it('keeps only the base name of what the clipboard calls the file', async () => {
        const path = await writeTempAttachment('../../escape.txt', new Uint8Array());

        expect(path).toMatch(/^\/tmp\/etch-paste-[^/]+\/escape\.txt$/);
    });

    it('removes the directory when the write fails', async () => {
        vi.mocked(writeFile).mockRejectedValueOnce(new Error('disk full'));

        await expect(writeTempAttachment('notes.pdf', new Uint8Array())).rejects.toThrow('disk full');

        const dir = vi.mocked(mkdir).mock.calls[0][0] as string;
        expect(remove).toHaveBeenCalledWith(dir, { recursive: true });
    });
});

describe('isTempPath', () => {
    it('recognises only a file inside an etch-paste directory', () => {
        expect(isTempPath('/tmp/etch-paste-abc/notes.pdf')).toBe(true);
        expect(isTempPath('C:\\Temp\\etch-paste-abc\\notes.pdf')).toBe(true);
        expect(isTempPath('/tmp/etch-paste-123.png')).toBe(false);
        expect(isTempPath('/home/user/my-etch-paste-dir/photo.png')).toBe(false);
        expect(isTempPath('/home/user/photo.png')).toBe(false);
    });
});

describe('discardTempFile', () => {
    beforeEach(() => {
        vi.mocked(remove).mockClear();
    });

    it('removes the whole etch-paste directory for the directory form', async () => {
        await discardTempFile('/tmp/etch-paste-abc/notes.pdf');

        expect(remove).toHaveBeenCalledWith('/tmp/etch-paste-abc', { recursive: true });
    });

    it('leaves a flat etch-paste file alone, since only the directory form is temp', async () => {
        await discardTempFile('/tmp/etch-paste-123.png');

        expect(remove).not.toHaveBeenCalled();
    });

    it('never touches a file outside the convention', async () => {
        await discardTempFile('/home/user/photo.png');

        expect(remove).not.toHaveBeenCalled();
    });
});
