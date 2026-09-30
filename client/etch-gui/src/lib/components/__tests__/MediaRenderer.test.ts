import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { resetStores } from '$lib/stores/__tests__/helpers';
import { activeOverlay, overlayImageUrl } from '$lib/stores/overlay';
import MediaRenderer from '../MediaRenderer.svelte';

const SRC = 'etch-media://example.org/abc123?mime=video%2Fmp4';

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
    resetStores();
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
    it('plays video inline from a blob of the fetched bytes, typed with its mimetype', async () => {
        const { container } = renderMedia({ mimetype: 'video/mp4', body: 'clip.mp4' });

        const video = container.querySelector('video')!;
        expect(video.hasAttribute('controls')).toBe(true);
        expect(video.getAttribute('preload')).toBe('metadata');
        expect(fetchMock).toHaveBeenCalledWith(SRC);
        await waitFor(() => expect(video.getAttribute('src')).toBe('blob:media-1'));
        expect(createdBlobs[0].type).toBe('video/mp4');
        expect(container.querySelector('img')).not.toBeInTheDocument();
        expect(screen.queryByText('clip.mp4')).not.toBeInTheDocument();
    });

    it('never hands the player an etch-media URL, since WebKitGTK cannot stream one', () => {
        const { container } = renderMedia({ mimetype: 'video/mp4', body: 'clip.mp4' });

        expect(container.querySelector('video')!.hasAttribute('src')).toBe(false);
    });

    it('offers a video the webview cannot play as a download and releases its blob', async () => {
        const { container } = renderMedia({ mimetype: 'video/x-matroska', body: 'clip.mkv', size: 1_572_864 });
        const video = container.querySelector('video')!;
        await waitFor(() => expect(video.getAttribute('src')).toBe('blob:media-1'));

        await fireEvent.error(video);

        expect(container.querySelector('video')).not.toBeInTheDocument();
        expect(container.querySelector('.file-download')).toBeInTheDocument();
        expect(screen.getByText('clip.mkv')).toBeInTheDocument();
        expect(screen.getByText('1.5 MB')).toBeInTheDocument();
        expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:media-1');
    });

    it('offers a video as a download when its bytes cannot be fetched', async () => {
        fetchMock.mockResolvedValueOnce({ ok: false, status: 502, arrayBuffer: async () => new ArrayBuffer(0) });
        const { container } = renderMedia({ mimetype: 'video/mp4', body: 'clip.mp4' });

        await waitFor(() => expect(container.querySelector('.file-download')).toBeInTheDocument());
        expect(container.querySelector('video')).not.toBeInTheDocument();
        expect(URL.createObjectURL).not.toHaveBeenCalled();
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

        await rerender({ src: 'etch-media://example.org/other?mime=video%2Fmp4' });
        const video = container.querySelector('video')!;
        await waitFor(() => expect(video.getAttribute('src')).toBe('blob:media-1'));
        finishFirst(okResponse());
        await new Promise((resolve) => setTimeout(resolve, 0));

        expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
        expect(video.getAttribute('src')).toBe('blob:media-1');
    });

    it('plays audio from a blob with its controls, name and size', async () => {
        const { container } = renderMedia({ mimetype: 'audio/ogg', body: 'voice.ogg', size: 48_128 });

        const audio = container.querySelector('audio')!;
        expect(audio.hasAttribute('controls')).toBe(true);
        expect(audio.getAttribute('preload')).toBe('metadata');
        await waitFor(() => expect(audio.getAttribute('src')).toBe('blob:media-1'));
        expect(createdBlobs[0].type).toBe('audio/ogg');
        expect(screen.getByText('voice.ogg')).toBeInTheDocument();
        expect(screen.getByText('47 KB')).toBeInTheDocument();
        expect(container.querySelector('video')).not.toBeInTheDocument();
    });

    it('shows a GIF as an image so it keeps animating', () => {
        const { container } = renderMedia({ mimetype: 'image/gif', body: 'party.gif' });

        expect(container.querySelector('img')?.getAttribute('src')).toBe(SRC);
        expect(container.querySelector('video')).not.toBeInTheDocument();
        expect(container.querySelector('.file-download')).not.toBeInTheDocument();
    });

    it('opens an image in the viewer when clicked', async () => {
        const { container } = renderMedia({ mimetype: 'image/png', body: 'photo.png' });

        await fireEvent.click(container.querySelector('.image-btn')!);

        expect(get(activeOverlay)).toBe('image');
        expect(get(overlayImageUrl)).toBe(SRC);
    });

    it('reserves the scaled size of an image before it loads', () => {
        const { container } = renderMedia({ mimetype: 'image/jpeg', body: 'photo.jpg', width: 1600, height: 1200 });

        const img = container.querySelector('img')!;
        expect(img.style.width).toBe('400px');
        expect(img.style.aspectRatio).toBe('1600 / 1200');
    });

    it('reserves the scaled size of a video before it loads', () => {
        const { container } = renderMedia({ mimetype: 'video/webm', body: 'clip.webm', width: 720, height: 1280 });

        const video = container.querySelector('video')!;
        expect(video.style.width).toBe('169px');
        expect(video.style.aspectRatio).toBe('720 / 1280');
    });

    it('reserves nothing when the dimensions are unknown', () => {
        const { container } = renderMedia({ mimetype: 'image/png', body: 'photo.png', width: 0, height: 0 });

        expect(container.querySelector('img')!.getAttribute('style')).toBeNull();
    });

    it('shows other files as a download card with a readable size', () => {
        const { container } = renderMedia({ mimetype: 'application/pdf', body: 'notes.pdf', size: 3_565_158 });

        expect(container.querySelector('.file-download')).toBeInTheDocument();
        expect(screen.getByText('notes.pdf')).toBeInTheDocument();
        expect(screen.getByText('3.4 MB')).toBeInTheDocument();
        expect(container.querySelector('img, video, audio')).not.toBeInTheDocument();
    });

    it('leaves the size off when it is unknown', () => {
        const { container } = renderMedia({ mimetype: 'application/zip', body: 'bundle.zip', size: 0 });

        expect(container.querySelector('.file-size')).not.toBeInTheDocument();
    });

    it('offers an SVG as a file, since it is never served as an image', () => {
        const { container } = renderMedia({ mimetype: 'image/svg+xml', body: 'logo.svg', size: 2048 });

        expect(container.querySelector('img')).not.toBeInTheDocument();
        expect(container.querySelector('.file-download')).toBeInTheDocument();
        expect(screen.getByText('2 KB')).toBeInTheDocument();
    });
});
