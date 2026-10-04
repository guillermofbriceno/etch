import { invoke } from '@tauri-apps/api/core';
import { readFile } from '@tauri-apps/plugin-fs';
import type { OutgoingMediaInfo } from './ipc';
import type { UploadLimits } from './stores/uploads';

/** Core's verdict on a file of this name and size; it decides again at send time with the session's real limits. */
export type Verdict =
    | { type: 'Accept'; compress_offered: boolean }
    | { type: 'MustCompress' }
    | { type: 'Reject'; reason: string };

export type Inspection = { mimetype: string; limit: number; verdict: Verdict };

export type AttachedFile = { path: string; size: number; inspection: Inspection };

const PROBE_TIMEOUT_MS = 3000;

// The probe reads the whole file into the webview, and nothing above the largest Etch cap can be sent anyway.
const PROBE_MAX_BYTES = 5 * 1024 ** 2;

export function fileName(path: string): string {
    return path.split(/[\\/]/).pop() || path;
}

export function inspectAttachment(name: string, size: number, limits: UploadLimits | null): Promise<Inspection> {
    return invoke<Inspection>('inspect_attachment', { name, size, limits });
}

function mediaInfo(width: number, height: number, seconds: number): OutgoingMediaInfo | null {
    const info = {
        width: width > 0 ? Math.round(width) : null,
        height: height > 0 ? Math.round(height) : null,
        duration_ms: Number.isFinite(seconds) && seconds > 0 ? Math.round(seconds * 1000) : null,
    };
    return info.width === null && info.height === null && info.duration_ms === null ? null : info;
}

function metadataLoaded(media: HTMLMediaElement): Promise<void> {
    return new Promise((resolve, reject) => {
        media.addEventListener('loadedmetadata', () => resolve(), { once: true });
        media.addEventListener('error', () => reject(new Error('media could not be decoded')), { once: true });
    });
}

/** Video and audio only: core reads an image's dimensions itself. */
export async function probeMediaInfo(path: string, size: number, mimetype: string): Promise<OutgoingMediaInfo | null> {
    const kind = mimetype.slice(0, mimetype.indexOf('/'));
    if (size > PROBE_MAX_BYTES || (kind !== 'video' && kind !== 'audio')) return null;

    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error('media probe timed out')), PROBE_TIMEOUT_MS);
    });
    let url: string | null = null;
    let media: HTMLMediaElement | null = null;

    try {
        const blob = new Blob([await Promise.race([readFile(path), deadline])], { type: mimetype });
        url = URL.createObjectURL(blob);
        media = document.createElement(kind);
        media.preload = 'metadata';
        media.muted = true;
        const ready = metadataLoaded(media);
        media.src = url;
        await Promise.race([ready, deadline]);
        return media instanceof HTMLVideoElement
            ? mediaInfo(media.videoWidth, media.videoHeight, media.duration)
            : mediaInfo(0, 0, media.duration);
    } catch {
        return null;
    } finally {
        clearTimeout(timer);
        media?.removeAttribute('src');
        if (url) URL.revokeObjectURL(url);
    }
}

/** The shell writes the file, so that Etch has a record of having created it. */
export async function writeTempAttachment(name: string, bytes: Uint8Array): Promise<string> {
    return invoke<string>('save_pasted_file', bytes, { headers: { 'file-name': encodeURIComponent(name) } });
}

/** Best effort. The shell only removes a file Etch itself created, so any path is safe to pass. */
export async function discardTempFile(path: string): Promise<void> {
    await invoke('discard_temp_upload', { path }).catch(() => undefined);
}
