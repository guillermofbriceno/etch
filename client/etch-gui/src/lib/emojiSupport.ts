import { versionProbes, type RenderSupport } from './emoji';

const SIZE = 24;
const SHOW_ALL: RenderSupport = { maxVersion: Infinity, flags: true };

let cached: RenderSupport | null = null;

/** Draws one probe per emoji version and keeps the versions the system font renders as a single color glyph. */
export function detectRenderSupport(): RenderSupport {
    if (cached) return cached;
    cached = probe() ?? SHOW_ALL;
    return cached;
}

function context(): CanvasRenderingContext2D | null {
    try {
        const canvas = document.createElement('canvas');
        canvas.width = SIZE * 3;
        canvas.height = SIZE;
        return canvas.getContext('2d', { willReadFrequently: true });
    } catch {
        return null;
    }
}

function probe(): RenderSupport | null {
    const ctx = context();
    if (!ctx) return null;
    ctx.font = `${SIZE * 0.75}px ${getComputedStyle(document.body).getPropertyValue('--font-family-emoji') || 'sans-serif'}`;
    ctx.textBaseline = 'top';
    ctx.fillStyle = '#000';

    const singleWidth = ctx.measureText('😀').width;
    const renders = (emoji: string): boolean => {
        if (ctx.measureText(emoji).width > singleWidth * 1.5) return false;
        ctx.clearRect(0, 0, SIZE * 3, SIZE);
        ctx.fillText(emoji, 0, 0);
        const px = ctx.getImageData(0, 0, SIZE * 3, SIZE).data;
        for (let i = 0; i < px.length; i += 4) {
            if (px[i + 3] > 0 && (px[i] !== px[i + 1] || px[i + 1] !== px[i + 2])) return true;
        }
        return false;
    };

    // If even the oldest emoji fails, the check itself is broken here, so show everything.
    if (!renders('😀')) return null;
    const maxVersion = versionProbes().find(([, emoji]) => renders(emoji))?.[0] ?? 1;
    return { maxVersion, flags: renders('🇺🇸') };
}
