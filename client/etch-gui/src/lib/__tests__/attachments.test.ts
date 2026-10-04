import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { readFile } from '@tauri-apps/plugin-fs';
import { probeMediaInfo } from '../attachments';

const MIB = 1024 * 1024;

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
        const probingVideo = probeMediaInfo('/tmp/clip.mp4', 1000, 'video/mp4');
        const video = await loadingElement('VIDEO');
        Object.defineProperty(video, 'videoWidth', { value: 1920 });
        Object.defineProperty(video, 'videoHeight', { value: 1080 });
        Object.defineProperty(video, 'duration', { value: 12.3456 });
        video.dispatchEvent(new Event('loadedmetadata'));

        expect(await probingVideo).toEqual({ width: 1920, height: 1080, duration_ms: 12346 });
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:probe');
        expect(video.hasAttribute('src')).toBe(false);

        const probingAudio = probeMediaInfo('/tmp/voice.ogg', 1000, 'audio/ogg');
        const audio = await loadingElement('AUDIO');
        Object.defineProperty(audio, 'duration', { value: 3.5 });
        audio.dispatchEvent(new Event('loadedmetadata'));

        expect(await probingAudio).toEqual({ width: null, height: null, duration_ms: 3500 });
    });

    it('gives up after a short timeout and still revokes the object url', async () => {
        vi.useFakeTimers();
        const pending = probeMediaInfo('/tmp/clip.mp4', 1000, 'video/mp4');

        await vi.advanceTimersByTimeAsync(3000);

        expect(await pending).toBeNull();
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:probe');
    });

    it('never reads an image, which core measures, a file with no media metadata, or one too large to send', async () => {
        expect(await probeMediaInfo('/tmp/photo.png', 1000, 'image/png')).toBeNull();
        expect(await probeMediaInfo('/tmp/notes.pdf', 1000, 'application/pdf')).toBeNull();
        expect(await probeMediaInfo('/tmp/clip.mp4', 50 * MIB, 'video/mp4')).toBeNull();
        expect(readFile).not.toHaveBeenCalled();
    });
});
