import { describe, it, expect } from 'vitest';
import { fitWithin, formatSize } from '../media';

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
