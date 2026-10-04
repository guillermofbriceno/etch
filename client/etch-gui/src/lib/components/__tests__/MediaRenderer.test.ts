import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, fireEvent, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import MediaRenderer from '../MediaRenderer.svelte';

const SRC = 'etch-media://example.org/abc123';

type Props = { src?: string; mimetype: string; body: string; size?: number; width?: number; height?: number };

function renderMedia(props: Props) {
    return render(MediaRenderer, { props: { src: SRC, ...props } });
}

function okResponse() {
    return { ok: true, status: 200, arrayBuffer: async () => new Uint8Array([0, 0, 0, 24]).buffer };
}

let fetchMock: ReturnType<typeof vi.fn>;
let createdBlobs: Blob[];

beforeEach(() => {
    fetchMock = vi.fn(async () => okResponse());
    vi.stubGlobal('fetch', fetchMock);
    createdBlobs = [];
    Object.defineProperty(URL, 'createObjectURL', {
        value: vi.fn((blob: Blob) => { createdBlobs.push(blob); return `blob:media-${createdBlobs.length}`; }),
        configurable: true, writable: true,
    });
    Object.defineProperty(URL, 'revokeObjectURL', { value: vi.fn(), configurable: true, writable: true });
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe('MediaRenderer', () => {
    // WebKitGTK cannot stream an etch-media URL, so a player must only ever be given the blob.
    it('plays video and audio inline from a blob of the fetched bytes, typed with the mimetype', async () => {
        const cases = [
            { tag: 'video', mimetype: 'video/mp4', body: 'clip.mp4' },
            { tag: 'audio', mimetype: 'audio/ogg', body: 'voice.ogg' },
        ];
        for (const [i, { tag, mimetype, body }] of cases.entries()) {
            const { container } = renderMedia({ mimetype, body });

            const player = container.querySelector(tag)!;
            expect(player.hasAttribute('src'), tag).toBe(false);
            expect(fetchMock).toHaveBeenLastCalledWith(SRC);
            await waitFor(() => expect(player.getAttribute('src')).toBe(`blob:media-${i + 1}`));
            expect(createdBlobs[i].type).toBe(mimetype);
            expect(container.querySelector('.file-download'), tag).not.toBeInTheDocument();
        }
    });

    it('offers a video the webview cannot play as a download and releases its blob', async () => {
        const { container } = renderMedia({ mimetype: 'video/x-matroska', body: 'clip.mkv' });
        const video = container.querySelector('video')!;
        await waitFor(() => expect(video.getAttribute('src')).toBe('blob:media-1'));

        await fireEvent.error(video);

        expect(container.querySelector('video')).not.toBeInTheDocument();
        expect(container.querySelector('.file-download')).toBeInTheDocument();
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:media-1');
    });

    it('offers a video as a download when its bytes cannot be fetched', async () => {
        fetchMock.mockResolvedValueOnce({ ok: false, status: 502, arrayBuffer: async () => new ArrayBuffer(0) });
        const { container } = renderMedia({ mimetype: 'video/mp4', body: 'clip.mp4' });

        await waitFor(() => expect(container.querySelector('.file-download')).toBeInTheDocument());
        expect(container.querySelector('video')).not.toBeInTheDocument();
    });

    it('offers video over the inline playback size as a download without fetching it', () => {
        const { container } = renderMedia({ mimetype: 'video/mp4', body: 'long.mp4', size: 20 * 1024 * 1024 + 1 });

        expect(container.querySelector('video')).not.toBeInTheDocument();
        expect(container.querySelector('.file-download')).toBeInTheDocument();
        expect(fetchMock).not.toHaveBeenCalled();
    });

    it('releases the blob when the message goes away', async () => {
        const { container, unmount } = renderMedia({ mimetype: 'video/mp4', body: 'clip.mp4' });
        await waitFor(() => expect(container.querySelector('video')!.getAttribute('src')).toBe('blob:media-1'));

        unmount();

        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:media-1');
    });

    it('ignores a fetch that finishes after the source changed', async () => {
        let finishFirst: (value: unknown) => void = () => {};
        fetchMock.mockImplementationOnce(() => new Promise((resolve) => { finishFirst = resolve; }));
        const { container, rerender } = renderMedia({ mimetype: 'video/mp4', body: 'clip.mp4' });

        await rerender({ src: 'etch-media://example.org/other' });
        const video = container.querySelector('video')!;
        await waitFor(() => expect(video.getAttribute('src')).toBe('blob:media-1'));
        finishFirst(okResponse());
        await new Promise((resolve) => setTimeout(resolve, 0));

        expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
        expect(video.getAttribute('src')).toBe('blob:media-1');
    });

    it('shows an image inline, but offers an SVG or any other file as a download', () => {
        const gif = renderMedia({ mimetype: 'image/gif', body: 'party.gif' }).container;
        expect(gif.querySelector('img')?.getAttribute('src')).toBe(SRC);

        for (const [mimetype, body] of [['image/svg+xml', 'logo.svg'], ['application/pdf', 'notes.pdf']]) {
            const { container } = renderMedia({ mimetype, body });
            expect(container.querySelector('img, video, audio'), mimetype).not.toBeInTheDocument();
            expect(container.querySelector('.file-download'), mimetype).toBeInTheDocument();
        }
    });

    // The webview may not write files, so the shell is given the bytes and asks where to put them.
    it('hands a download to the shell with its bytes and its name', async () => {
        const { container } = renderMedia({ mimetype: 'application/pdf', body: 'résumé.pdf' });

        await fireEvent.click(container.querySelector('.file-download')!);

        await waitFor(() => expect(invoke).toHaveBeenCalledWith(
            'save_file_as',
            new Uint8Array([0, 0, 0, 24]),
            { headers: { 'file-name': 'r%C3%A9sum%C3%A9.pdf' } },
        ));
    });

    it('reserves the scaled size of media before it loads, when the dimensions are known', () => {
        const known = renderMedia({ mimetype: 'image/jpeg', body: 'photo.jpg', width: 1600, height: 1200 }).container;
        const img = known.querySelector('img')!;
        expect(img.style.width).toBe('400px');
        expect(img.style.aspectRatio).toBe('1600 / 1200');

        const unknown = renderMedia({ mimetype: 'image/png', body: 'photo.png', width: 0, height: 0 }).container;
        expect(unknown.querySelector('img')!.getAttribute('style')).toBeNull();
    });
});
