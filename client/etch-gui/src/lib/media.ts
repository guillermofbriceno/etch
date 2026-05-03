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
