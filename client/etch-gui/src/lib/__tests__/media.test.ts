import { describe, it, expect } from 'vitest';
import { fitWithin, formatMB, formatSize } from '../media';

describe('formatMB', () => {
    // The same sizes as core's format_mb test, because the two texts must agree byte for byte.
    it('rounds to tenths, half up, and drops the decimal only for a whole number of MiB', () => {
        expect(formatMB(2_097_152)).toBe('2 MB');
        expect(formatMB(2_097_153)).toBe('2.0 MB');
        expect(formatMB(1_310_720)).toBe('1.3 MB');
        expect(formatMB(2_359_296)).toBe('2.3 MB');
        expect(formatMB(3_565_158)).toBe('3.4 MB');
        expect(formatMB(50_000_000)).toBe('47.7 MB');
    });
});

describe('formatSize', () => {
    it('uses the largest unit that fits and never shows 1024 of a smaller one', () => {
        expect(formatSize(0)).toBe('0 B');
        expect(formatSize(1023)).toBe('1023 B');
        expect(formatSize(1024)).toBe('1 KB');
        expect(formatSize(1536)).toBe('1.5 KB');
        expect(formatSize(1_048_575)).toBe('1.0 MB');
        expect(formatSize(3 * 1024 ** 3)).toBe('3 GB');
    });
});

describe('fitWithin', () => {
    it('scales down to whichever bound binds, never up, and gives null for an unknown dimension', () => {
        expect(fitWithin(1000, 300, 400, 300)).toEqual({ width: 400, height: 120 });
        expect(fitWithin(300, 1200, 400, 300)).toEqual({ width: 75, height: 300 });
        expect(fitWithin(200, 100, 400, 300)).toEqual({ width: 200, height: 100 });
        expect(fitWithin(0, 300, 400, 300)).toBeNull();
        expect(fitWithin(300, 0, 400, 300)).toBeNull();
    });
});
