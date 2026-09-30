import { describe, it, expect } from 'vitest';
import { resolveMediaUrl, resolveMessageMediaUrl, fitWithin, formatMB, formatSize } from '../media';

describe('resolveMessageMediaUrl', () => {
    it('adds the percent-encoded mimetype as a hint', () => {
        expect(resolveMessageMediaUrl('mxc://example.org/abc123', 'video/mp4'))
            .toBe('etch-media://example.org/abc123?mime=video%2Fmp4');
    });

    it('encodes parameters inside the mimetype', () => {
        expect(resolveMessageMediaUrl('mxc://example.org/abc123', 'audio/ogg; codecs=opus'))
            .toBe('etch-media://example.org/abc123?mime=audio%2Fogg%3B%20codecs%3Dopus');
    });

    it('adds no hint without a mimetype', () => {
        expect(resolveMessageMediaUrl('mxc://example.org/abc123', '')).toBe('etch-media://example.org/abc123');
    });

    it('leaves a non-mxc url alone', () => {
        expect(resolveMessageMediaUrl('https://example.org/a.png', 'image/png')).toBe('https://example.org/a.png');
    });

    it('returns null for a missing url', () => {
        expect(resolveMessageMediaUrl(null, 'image/png')).toBeNull();
    });

    it('does not change avatar urls', () => {
        expect(resolveMediaUrl('mxc://example.org/avatar')).toBe('etch-media://example.org/avatar');
    });
});

describe('formatMB', () => {
    it('shows an exact multiple of a MiB as a whole number', () => {
        expect(formatMB(2_097_152)).toBe('2 MB');
        expect(formatMB(5_242_880)).toBe('5 MB');
    });

    it('rounds a half tenth up, exactly as core does', () => {
        expect(formatMB(1_310_720)).toBe('1.3 MB');
        expect(formatMB(2_359_296)).toBe('2.3 MB');
    });

    it('shows one decimal for anything else, even when it rounds to whole', () => {
        expect(formatMB(2_097_153)).toBe('2.0 MB');
        expect(formatMB(3_565_158)).toBe('3.4 MB');
        expect(formatMB(1_468_006)).toBe('1.4 MB');
    });
});

describe('formatSize', () => {
    it('keeps small sizes in bytes', () => {
        expect(formatSize(0)).toBe('0 B');
        expect(formatSize(1023)).toBe('1023 B');
    });

    it('uses the largest unit that fits', () => {
        expect(formatSize(1024)).toBe('1 KB');
        expect(formatSize(1536)).toBe('1.5 KB');
        expect(formatSize(1_048_576)).toBe('1 MB');
        expect(formatSize(3_565_158)).toBe('3.4 MB');
        expect(formatSize(3 * 1024 ** 3)).toBe('3 GB');
    });

    it('never shows 1024 of a smaller unit', () => {
        expect(formatSize(1_048_575)).toBe('1.0 MB');
    });
});

describe('fitWithin', () => {
    it('scales a large landscape image to the width bound', () => {
        expect(fitWithin(1000, 300, 400, 300)).toEqual({ width: 400, height: 120 });
    });

    it('scales a tall image to the height bound', () => {
        expect(fitWithin(300, 1200, 400, 300)).toEqual({ width: 75, height: 300 });
    });

    it('keeps the aspect ratio when both bounds bind', () => {
        expect(fitWithin(1600, 1200, 400, 300)).toEqual({ width: 400, height: 300 });
    });

    it('never scales up', () => {
        expect(fitWithin(200, 100, 400, 300)).toEqual({ width: 200, height: 100 });
    });

    it('returns null when a dimension is unknown', () => {
        expect(fitWithin(0, 300, 400, 300)).toBeNull();
        expect(fitWithin(300, 0, 400, 300)).toBeNull();
    });
});
