import type { NameColor } from './types';

const LIGHTNESS = 0.76;
const CHROMA = 0.122;

const HEX_COLOR = /^#[0-9a-f]{6}$/i;
const MIN_CONTRAST = 4.5;
// The lighter of the two themes' chat backgrounds, so a color readable here is readable on both.
const CHAT_BACKGROUND = '#121212';

export const AUTOMATIC_HUES: readonly number[] = Array.from({ length: 10 }, (_, i) => 25 + 36 * i);

export function automaticHue(userId: string): number {
    let hash = 0x811c9dc5;
    for (let i = 0; i < userId.length; i++) {
        hash ^= userId.charCodeAt(i);
        hash = Math.imul(hash, 0x01000193);
    }
    return AUTOMATIC_HUES[(hash >>> 0) % AUTOMATIC_HUES.length];
}

/** The gamma-encoded sRGB channels of a name color, before clamping to 0..1. */
function hueToSrgb(hue: number): [number, number, number] {
    const radians = (hue * Math.PI) / 180;
    const a = CHROMA * Math.cos(radians);
    const b = CHROMA * Math.sin(radians);
    const l = (LIGHTNESS + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const m = (LIGHTNESS - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const s = (LIGHTNESS - 0.0894841775 * a - 1.291485548 * b) ** 3;
    return [
        gammaEncode(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
        gammaEncode(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
        gammaEncode(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
    ];
}

function gammaEncode(linear: number): number {
    return linear <= 0.0031308 ? 12.92 * linear : 1.055 * linear ** (1 / 2.4) - 0.055;
}

function gammaDecode(encoded: number): number {
    return encoded <= 0.04045 ? encoded / 12.92 : ((encoded + 0.055) / 1.055) ** 2.4;
}

function hexByte(channel: number): string {
    return Math.round(Math.min(1, Math.max(0, channel)) * 255).toString(16).padStart(2, '0');
}

// Hex rather than CSS oklch(), which not every WebKitGTK the app runs on is known to support.
export function hueColor(hue: number): string {
    return `#${hueToSrgb(hue).map(hexByte).join('')}`;
}

function linearRgb(hex: string): [number, number, number] {
    const n = parseInt(hex.slice(1), 16);
    return [gammaDecode((n >> 16) / 255), gammaDecode(((n >> 8) & 0xff) / 255), gammaDecode((n & 0xff) / 255)];
}

function relativeLuminance(hex: string): number {
    const [r, g, b] = linearRgb(hex);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function isReadable(color: string): boolean {
    if (!HEX_COLOR.test(color)) return false;
    const [lighter, darker] = [relativeLuminance(color), relativeLuminance(CHAT_BACKGROUND)].sort((x, y) => y - x);
    return (lighter + 0.05) / (darker + 0.05) >= MIN_CONTRAST;
}

/** Typed input as lowercase `#rrggbb`, with the `#` optional, or null when it is not a color. */
export function parseHex(input: string): string | null {
    const hex = input.trim().replace(/^#?/, '#').toLowerCase();
    return HEX_COLOR.test(hex) ? hex : null;
}

/** The OKLCH hue of a `#rrggbb` color, in whole degrees. */
export function hexHue(hex: string): number {
    const [red, green, blue] = linearRgb(hex);
    const l = Math.cbrt(0.4122214708 * red + 0.5363325363 * green + 0.0514459929 * blue);
    const m = Math.cbrt(0.2119034982 * red + 0.6806995451 * green + 0.1073969566 * blue);
    const s = Math.cbrt(0.0883024619 * red + 0.2817104115 * green + 0.6299787211 * blue);
    const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    const b = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    return Math.round((Math.atan2(b, a) * 180) / Math.PI + 360) % 360;
}

export function userColor(userId: string, chosen: NameColor | null): string {
    return chosen && isReadable(chosen.color) ? chosen.color : hueColor(automaticHue(userId));
}
