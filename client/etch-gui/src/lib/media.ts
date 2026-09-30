import { PLATFORM_WINDOWS } from './platform';

/** Convert an mxc:// URL to the app's etch-media:// protocol. Non-mxc URLs pass through unchanged.
 *  On Windows (WebView2), custom schemes are served via https://<scheme>.localhost/. */
export function resolveMediaUrl(url: string | null | undefined): string | null {
    if (!url) return null;
    if (url.startsWith('mxc://')) {
        const path = url.slice('mxc://'.length);
        if (PLATFORM_WINDOWS) {
            return `http://etch-media.localhost/${path}`;
        }
        return `etch-media://${path}`;
    }
    return url;
}

/** Message media carries its mimetype as a hint so the protocol handler can serve a playable Content-Type. */
export function resolveMessageMediaUrl(url: string | null | undefined, mimetype: string | null | undefined): string | null {
    const resolved = resolveMediaUrl(url);
    if (!resolved || !url?.startsWith('mxc://') || !mimetype) return resolved;
    return `${resolved}?mime=${encodeURIComponent(mimetype)}`;
}

export function fitWithin(width: number, height: number, maxWidth: number, maxHeight: number): { width: number; height: number } | null {
    if (!(width > 0) || !(height > 0)) return null;
    const scale = Math.min(1, maxWidth / width, maxHeight / height);
    return { width: Math.round(width * scale), height: Math.round(height * scale) };
}

const SIZE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

// Integer rounding to tenths, half up, so the text matches core's reasons byte for byte.
function inUnit(bytes: number, unitBytes: number): string {
    if (bytes % unitBytes === 0) return String(bytes / unitBytes);
    const tenths = Math.floor((bytes * 10 + unitBytes / 2) / unitBytes);
    return `${Math.floor(tenths / 10)}.${tenths % 10}`;
}

export function formatMB(bytes: number): string {
    return `${inUnit(bytes, 1024 ** 2)} MB`;
}

export function formatSize(bytes: number): string {
    let unit = 0;
    // Moves up early enough that a size never reads as 1024.0 of the smaller unit.
    while (unit < SIZE_UNITS.length - 1 && bytes / 1024 ** unit >= 1023.95) unit++;
    return unit === 0 ? `${bytes} B` : `${inUnit(bytes, 1024 ** unit)} ${SIZE_UNITS[unit]}`;
}

/** Extract the first visible character from a display name for avatar fallbacks. Strips a leading '@' (Matrix IDs). */
export function getInitial(name: string | null | undefined, fallback = '?'): string {
    if (!name) return fallback;
    const stripped = name.startsWith('@') ? name.slice(1) : name;
    return (stripped.charAt(0) || fallback).toUpperCase();
}

export async function fetchBlob(url: string): Promise<Uint8Array> {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`Fetch failed (${res.status})`);
    return new Uint8Array(await res.arrayBuffer());
}
