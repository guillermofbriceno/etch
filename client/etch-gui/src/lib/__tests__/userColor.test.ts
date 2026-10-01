import { describe, it, expect } from 'vitest';
import { AUTOMATIC_HUES, automaticHue, hueColor, isReadable, userColor } from '../userColor';

function srgb(hex: string): [number, number, number] {
    const n = parseInt(hex.slice(1), 16);
    return [(n >> 16) / 255, ((n >> 8) & 0xff) / 255, (n & 0xff) / 255];
}

const linear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);

function contrastRatio(a: string, b: string): number {
    const lum = (hex: string) => {
        const [r, g, bl] = srgb(hex).map(linear);
        return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
    };
    const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
}

function oklab(hex: string): [number, number, number] {
    const [r, g, b] = srgb(hex).map(linear);
    const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
    const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
    const s = Math.cbrt(0.0883024619 * r + 0.2817104115 * g + 0.6299787211 * b);
    return [
        0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ];
}

describe('isReadable', () => {
    it.each([
        ['#7c7c7c', '#7d7d7d'],
        ['#f90000', '#fa0000'],
        ['#009000', '#009100'],
    ])('refuses %s and accepts %s, either side of 4.5:1 on the chat background', (below, above) => {
        expect(contrastRatio(below, '#121212')).toBeLessThan(4.5);
        expect(contrastRatio(above, '#121212')).toBeGreaterThanOrEqual(4.5);

        expect(isReadable(below)).toBe(false);
        expect(isReadable(above)).toBe(true);
    });

    it('accepts every color the hue slider can produce', () => {
        for (let hue = 0; hue < 360; hue++) {
            expect(isReadable(hueColor(hue)), `hue ${hue}`).toBe(true);
        }
    });
});

describe('automatic hues', () => {
    it('are pairwise perceptually distinct', () => {
        const colors = AUTOMATIC_HUES.map(hueColor);
        for (let i = 0; i < colors.length; i++) {
            for (let j = i + 1; j < colors.length; j++) {
                const [a, b] = [oklab(colors[i]), oklab(colors[j])];
                const distance = Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
                expect(distance, `${colors[i]} vs ${colors[j]}`).toBeGreaterThanOrEqual(0.07);
            }
        }
    });
});

describe('userColor', () => {
    it('shows a chosen color only when it is readable, and the automatic color otherwise', () => {
        const ALICE = '@alice:example.org';
        const automatic = hueColor(automaticHue(ALICE));
        expect(automatic).not.toBe('#ff8800');

        expect(userColor(ALICE, { color: '#ff8800' })).toBe('#ff8800');
        expect(userColor(ALICE, { color: '#7c7c7c' })).toBe(automatic);
        expect(userColor(ALICE, null)).toBe(automatic);
    });
});
