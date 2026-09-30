import { readFile, writeFile, mkdir, remove } from '@tauri-apps/plugin-fs';
import { tempDir, join } from '@tauri-apps/api/path';
import type { OutgoingMediaInfo } from './ipc';
import type { UploadLimits } from './stores/uploads';
import { formatMB } from './media';

export type AttachmentCategory = 'image' | 'other';

export type AttachVerdict = { accept: true; mustCompress: boolean } | { accept: false; reason: string };

export const COMPRESS_THRESHOLD_BYTES = 256_000;

const TEMP_PREFIX = 'etch-paste-';

const PROBE_TIMEOUT_MS = 3000;

// The probe reads the whole file into the webview, and nothing above the largest Etch cap can be sent anyway.
const PROBE_MAX_BYTES = 5 * 1024 ** 2;

const MIME_BY_EXTENSION: Record<string, string> = {
    png: 'image/png',
    apng: 'image/apng',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    jpe: 'image/jpeg',
    jfif: 'image/jpeg',
    gif: 'image/gif',
    webp: 'image/webp',
    bmp: 'image/bmp',
    dib: 'image/bmp',
    tif: 'image/tiff',
    tiff: 'image/tiff',
    avif: 'image/avif',
    heic: 'image/heic',
    heif: 'image/heif',
    jxl: 'image/jxl',
    ico: 'image/x-icon',
    svg: 'image/svg+xml',
    mp4: 'video/mp4',
    m4v: 'video/mp4',
    webm: 'video/webm',
    mov: 'video/quicktime',
    mkv: 'video/x-matroska',
    ogv: 'video/ogg',
    avi: 'video/x-msvideo',
    mpg: 'video/mpeg',
    mpeg: 'video/mpeg',
    mp3: 'audio/mpeg',
    m4a: 'audio/mp4',
    aac: 'audio/aac',
    ogg: 'audio/ogg',
    oga: 'audio/ogg',
    opus: 'audio/ogg',
    wav: 'audio/wav',
    flac: 'audio/flac',
    weba: 'audio/webm',
    pdf: 'application/pdf',
    txt: 'text/plain',
    md: 'text/markdown',
    csv: 'text/csv',
    json: 'application/json',
    rtf: 'application/rtf',
    epub: 'application/epub+zip',
    zip: 'application/zip',
    '7z': 'application/x-7z-compressed',
    rar: 'application/vnd.rar',
    tar: 'application/x-tar',
    gz: 'application/gzip',
    doc: 'application/msword',
    docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    xls: 'application/vnd.ms-excel',
    xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    ppt: 'application/vnd.ms-powerpoint',
    pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
    odt: 'application/vnd.oasis.opendocument.text',
    ods: 'application/vnd.oasis.opendocument.spreadsheet',
    odp: 'application/vnd.oasis.opendocument.presentation',
};

// Exactly what the shell's compress_image re-encodes; it hands anything else back unchanged.
const COMPRESSIBLE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'jpe', 'jfif', 'webp', 'bmp', 'dib', 'tif', 'tiff']);

export function fileName(path: string): string {
    return path.split(/[\\/]/).pop() || path;
}

function extension(path: string): string | null {
    const name = fileName(path);
    const dot = name.lastIndexOf('.');
    return dot > 0 ? name.slice(dot + 1).toLowerCase() : null;
}

/** Null for an extension this table does not know, where core's own guess may still call it an image. */
export function mimeFromPath(path: string): string | null {
    const ext = extension(path);
    if (ext === null) return 'application/octet-stream';
    return MIME_BY_EXTENSION[ext] ?? null;
}

export function categoryOf(mimetype: string | null): AttachmentCategory {
    return mimetype?.startsWith('image/') && mimetype !== 'image/gif' ? 'image' : 'other';
}

export function isCompressible(path: string): boolean {
    const ext = extension(path);
    return ext !== null && COMPRESSIBLE_EXTENSIONS.has(ext);
}

/** Null when the limits or the file's category are unknown, which leaves the check to core. */
export function limitFor(path: string, limits: UploadLimits | null): number | null {
    const mime = mimeFromPath(path);
    if (!limits || mime === null) return null;
    return categoryOf(mime) === 'image' ? limits.image_bytes : limits.other_bytes;
}

export function isTempPath(path: string): boolean {
    const parts = path.split(/[\\/]/);
    return parts.length > 1 && parts[parts.length - 2].startsWith(TEMP_PREFIX);
}

export function overLimitReason(size: number, limit: number, afterCompression = false): string {
    const when = afterCompression ? ' after compression' : '';
    return `it is ${formatMB(size)}${when} and the limit for this kind of file is ${formatMB(limit)}`;
}

export function compressionFailedReason(limit: number): string {
    return `it could not be compressed to fit the ${formatMB(limit)} limit`;
}

export function checkAttachment(path: string, size: number, limits: UploadLimits | null): AttachVerdict {
    const limit = limitFor(path, limits);
    if (limit === null || size <= limit) return { accept: true, mustCompress: false };
    if (isCompressible(path)) return { accept: true, mustCompress: true };
    return { accept: false, reason: overLimitReason(size, limit) };
}

function mediaInfo(width: number, height: number, seconds: number): OutgoingMediaInfo | null {
    const info = {
        width: width > 0 ? Math.round(width) : null,
        height: height > 0 ? Math.round(height) : null,
        duration_ms: Number.isFinite(seconds) && seconds > 0 ? Math.round(seconds * 1000) : null,
    };
    return info.width === null && info.height === null && info.duration_ms === null ? null : info;
}

function loaded(element: HTMLElement, event: 'load' | 'loadedmetadata'): Promise<void> {
    return new Promise((resolve, reject) => {
        element.addEventListener(event, () => resolve(), { once: true });
        element.addEventListener('error', () => reject(new Error('media could not be decoded')), { once: true });
    });
}

export async function probeMediaInfo(path: string, size: number): Promise<OutgoingMediaInfo | null> {
    const mime = mimeFromPath(path);
    const kind = mime?.slice(0, mime.indexOf('/'));
    if (!mime || size > PROBE_MAX_BYTES || (kind !== 'image' && kind !== 'video' && kind !== 'audio')) return null;

    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error('media probe timed out')), PROBE_TIMEOUT_MS);
    });
    let url: string | null = null;
    let element: HTMLImageElement | HTMLMediaElement | null = null;

    try {
        const blob = new Blob([await Promise.race([readFile(path), deadline])], { type: mime });

        if (kind === 'image' && typeof createImageBitmap === 'function') {
            const decoding = createImageBitmap(blob);
            const bitmap = await Promise.race([decoding, deadline]).catch((e) => {
                decoding.then(late => late.close(), () => undefined);
                throw e;
            });
            const info = mediaInfo(bitmap.width, bitmap.height, NaN);
            bitmap.close();
            return info;
        }

        url = URL.createObjectURL(blob);

        if (kind === 'image') {
            const image = element = new Image();
            const ready = loaded(image, 'load');
            image.src = url;
            await Promise.race([ready, deadline]);
            return mediaInfo(image.naturalWidth, image.naturalHeight, NaN);
        }

        const media = element = document.createElement(kind);
        media.preload = 'metadata';
        media.muted = true;
        const ready = loaded(media, 'loadedmetadata');
        media.src = url;
        await Promise.race([ready, deadline]);
        return media instanceof HTMLVideoElement
            ? mediaInfo(media.videoWidth, media.videoHeight, media.duration)
            : mediaInfo(0, 0, media.duration);
    } catch {
        return null;
    } finally {
        clearTimeout(timer);
        element?.removeAttribute('src');
        if (url) URL.revokeObjectURL(url);
    }
}

function safeFileName(name: string): string {
    const base = fileName(name).trim();
    return base && base !== '.' && base !== '..' ? base : 'attachment';
}

export async function writeTempAttachment(name: string, bytes: Uint8Array): Promise<string> {
    const dir = await join(await tempDir(), `${TEMP_PREFIX}${crypto.randomUUID()}`);
    await mkdir(dir, { recursive: true });
    const path = await join(dir, safeFileName(name));
    try {
        await writeFile(path, bytes);
    } catch (e) {
        await remove(dir, { recursive: true }).catch(() => undefined);
        throw e;
    }
    return path;
}

/** Best effort; only ever touches a file inside an etch-paste directory, and removes that directory. */
export async function discardTempFile(path: string): Promise<void> {
    if (!isTempPath(path)) return;
    const dir = path.slice(0, path.length - fileName(path).length - 1);
    await remove(dir, { recursive: true }).catch(() => undefined);
}
