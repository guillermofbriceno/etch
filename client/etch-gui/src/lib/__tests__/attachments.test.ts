import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { readFile } from '@tauri-apps/plugin-fs';
import { checkAttachment, probeMediaInfo } from '../attachments';

const MIB = 1024 * 1024;
const LIMITS = { image_bytes: 5 * MIB, other_bytes: 2 * MIB };

describe('checkAttachment', () => {
    const ACCEPT = { accept: true, mustCompress: false };
    const reject = (size: string, limit: string) => ({
        accept: false,
        reason: `it is ${size} and the limit for this kind of file is ${limit}`,
    });

    it('holds each kind of file to its limit, and leaves what it cannot judge to core', () => {
        const cases: [string, string, number, typeof LIMITS | null, unknown][] = [
            ['an other file at its limit', '/tmp/clip.mp4', 2 * MIB, LIMITS, ACCEPT],
            ['an other file one byte over', '/tmp/clip.mp4', 2 * MIB + 1, LIMITS, reject('2.0 MB', '2 MB')],
            ['a GIF, which is not held to the image limit', '/tmp/party.gif', 3 * MIB, LIMITS, reject('3 MB', '2 MB')],
            ['an image at its limit', '/tmp/photo.png', 5 * MIB, LIMITS, ACCEPT],
            ['an oversize image the shell can compress', 'C:\\Users\\user\\Photo.JPG', 9 * MIB, LIMITS, { accept: true, mustCompress: true }],
            ['an oversize image the shell cannot compress', '/tmp/photo.heic', 7 * MIB, LIMITS, reject('7 MB', '5 MB')],
            ['a file with no extension, a plain file as in core', '/home/user/README', 3 * MIB, LIMITS, reject('3 MB', '2 MB')],
            ['an extension only core can judge', '/home/user/raw.dng', 3 * MIB, LIMITS, ACCEPT],
            ['anything while the limits are unknown', '/tmp/huge.mp4', 900 * MIB, null, ACCEPT],
        ];
        for (const [label, path, size, limits, verdict] of cases) {
            expect(checkAttachment(path, size, limits), label).toEqual(verdict);
        }
    });
});

describe('probeMediaInfo', () => {
    let createdElements: HTMLElement[];
    // Restored on its own: restoreAllMocks would also strip the shared Tauri mocks of their defaults.
    let createElement: { mockRestore: () => void };

    beforeEach(() => {
        vi.mocked(readFile).mockReset();
        vi.mocked(readFile).mockResolvedValue(new Uint8Array([1, 2, 3]));
        Object.defineProperty(URL, 'createObjectURL', { value: vi.fn(() => 'blob:probe'), configurable: true, writable: true });
        Object.defineProperty(URL, 'revokeObjectURL', { value: vi.fn(), configurable: true, writable: true });
        createdElements = [];
        const create = document.createElement.bind(document);
        createElement = vi.spyOn(document, 'createElement').mockImplementation(((tag: string, options?: ElementCreationOptions) => {
            const element = create(tag, options);
            createdElements.push(element);
            return element;
        }) as typeof document.createElement);
    });

    afterEach(() => {
        vi.mocked(readFile).mockReset();
        vi.mocked(readFile).mockRejectedValue(new Error('no such file'));
        createElement.mockRestore();
        vi.useRealTimers();
    });

    async function loadingElement(tag: 'VIDEO' | 'AUDIO'): Promise<HTMLMediaElement> {
        await vi.waitFor(() => {
            expect(createdElements.some(e => e.tagName === tag && e.getAttribute('src') === 'blob:probe')).toBe(true);
        });
        return createdElements.find(e => e.tagName === tag) as HTMLMediaElement;
    }

    it('reads video dimensions and media durations from the loaded metadata, then lets go of the file', async () => {
        const probingVideo = probeMediaInfo('/tmp/clip.mp4', 1000);
        const video = await loadingElement('VIDEO');
        Object.defineProperty(video, 'videoWidth', { value: 1920 });
        Object.defineProperty(video, 'videoHeight', { value: 1080 });
        Object.defineProperty(video, 'duration', { value: 12.3456 });
        video.dispatchEvent(new Event('loadedmetadata'));

        expect(await probingVideo).toEqual({ width: 1920, height: 1080, duration_ms: 12346 });
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:probe');
        expect(video.hasAttribute('src')).toBe(false);

        const probingAudio = probeMediaInfo('/tmp/voice.ogg', 1000);
        const audio = await loadingElement('AUDIO');
        Object.defineProperty(audio, 'duration', { value: 3.5 });
        audio.dispatchEvent(new Event('loadedmetadata'));

        expect(await probingAudio).toEqual({ width: null, height: null, duration_ms: 3500 });
    });

    it('gives up after a short timeout and still revokes the object url', async () => {
        vi.useFakeTimers();
        const pending = probeMediaInfo('/tmp/clip.mp4', 1000);

        await vi.advanceTimersByTimeAsync(3000);

        expect(await pending).toBeNull();
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:probe');
    });

    it('never reads a file that carries no media metadata or is too large to send', async () => {
        expect(await probeMediaInfo('/tmp/notes.pdf', 1000)).toBeNull();
        expect(await probeMediaInfo('/tmp/clip.mp4', 50 * MIB)).toBeNull();
        expect(readFile).not.toHaveBeenCalled();
    });
});
