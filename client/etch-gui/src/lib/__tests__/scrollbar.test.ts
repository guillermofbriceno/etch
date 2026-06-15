import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
    customScrollbar,
    thumbGeometry,
    thumbTopForScroll,
    scrollTopForDrag,
    scrollTopForTrackClick,
} from '../scrollbar';

// The geometry helpers are the heart of the scrollbar and are pure, so they can
// be tested precisely. jsdom has no layout engine, so the DOM-level tests below
// only assert structure and lifecycle, never measured pixel values.

describe('thumbGeometry', () => {
    const opts = { trackPadding: 0, minThumbHeight: 0 };

    it('sizes the thumb proportionally to the visible fraction', () => {
        // Half the content is visible → thumb fills half the track.
        const g = thumbGeometry(200, 100, opts);
        expect(g.trackHeight).toBe(100);
        expect(g.thumbHeight).toBe(50);
        expect(g.thumbRange).toBe(50);
        expect(g.maxScrollTop).toBe(100);
    });

    it('subtracts track padding from both ends', () => {
        const g = thumbGeometry(200, 100, { trackPadding: 10, minThumbHeight: 0 });
        expect(g.trackHeight).toBe(80);
        expect(g.thumbHeight).toBe(40);
    });

    it('clamps the thumb up to the minimum height for tall content', () => {
        const g = thumbGeometry(10_000, 100, { trackPadding: 0, minThumbHeight: 30 });
        expect(g.thumbHeight).toBe(30); // raw would be ~1px
        expect(g.thumbRange).toBe(70);
    });

    it('never lets the thumb exceed the track height', () => {
        const g = thumbGeometry(200, 100, { trackPadding: 0, minThumbHeight: 500 });
        expect(g.thumbHeight).toBe(100);
        expect(g.thumbRange).toBe(0);
    });

    it('reports no scroll range when content fits', () => {
        const g = thumbGeometry(100, 100, opts);
        expect(g.maxScrollTop).toBe(0);
        expect(g.thumbRange).toBe(0);
    });
});

describe('thumbTopForScroll', () => {
    const geom = thumbGeometry(200, 100, { trackPadding: 0, minThumbHeight: 0 }); // range 50, max 100

    it('sits at the padding offset when scrolled to the top', () => {
        expect(thumbTopForScroll(0, geom, 0)).toBe(0);
    });

    it('sits at the bottom of its range when scrolled to the bottom', () => {
        expect(thumbTopForScroll(100, geom, 0)).toBe(50);
    });

    it('maps the midpoint linearly', () => {
        expect(thumbTopForScroll(50, geom, 0)).toBe(25);
    });

    it('adds the track padding to the offset', () => {
        const padded = thumbGeometry(200, 100, { trackPadding: 10, minThumbHeight: 0 });
        expect(thumbTopForScroll(0, padded, 10)).toBe(10);
        expect(thumbTopForScroll(100, padded, 10)).toBe(50); // 10 + range(40)
    });

    it('clamps over-scroll instead of overshooting the track', () => {
        expect(thumbTopForScroll(99999, geom, 0)).toBe(50);
    });

    it('returns just the padding when there is no scroll range', () => {
        const fits = thumbGeometry(100, 100, { trackPadding: 5, minThumbHeight: 0 });
        expect(thumbTopForScroll(0, fits, 5)).toBe(5);
    });
});

describe('scrollTopForDrag', () => {
    const geom = thumbGeometry(200, 100, { trackPadding: 0, minThumbHeight: 0 }); // range 50, max 100

    it('translates a full-range thumb drag into a full content scroll', () => {
        expect(scrollTopForDrag(0, 50, geom)).toBe(100);
    });

    it('scales partial drags proportionally', () => {
        expect(scrollTopForDrag(0, 25, geom)).toBe(50);
    });

    it('handles upward (negative) drags from a mid scroll position', () => {
        expect(scrollTopForDrag(20, -10, geom)).toBe(0);
    });

    it('is a no-op when the thumb cannot move', () => {
        const pinned = thumbGeometry(100, 100, { trackPadding: 0, minThumbHeight: 0 });
        expect(scrollTopForDrag(42, 100, pinned)).toBe(42);
    });
});

describe('scrollTopForTrackClick', () => {
    const geom = thumbGeometry(200, 100, { trackPadding: 0, minThumbHeight: 0 }); // track 100, max 100

    it('maps a click position to the same scroll fraction', () => {
        expect(scrollTopForTrackClick(0, geom)).toBe(0);
        expect(scrollTopForTrackClick(50, geom)).toBe(50);
        expect(scrollTopForTrackClick(100, geom)).toBe(100);
    });

    it('clamps clicks outside the track bounds', () => {
        expect(scrollTopForTrackClick(-20, geom)).toBe(0);
        expect(scrollTopForTrackClick(150, geom)).toBe(100);
    });

    it('returns 0 when the track has no height', () => {
        const empty = thumbGeometry(0, 0, { trackPadding: 0, minThumbHeight: 0 });
        expect(scrollTopForTrackClick(10, empty)).toBe(0);
    });
});

describe('customScrollbar action', () => {
    let parent: HTMLDivElement;
    let node: HTMLDivElement;

    // ResizeObserver is stubbed globally in test-setup.ts.
    beforeEach(() => {
        parent = document.createElement('div');
        parent.style.position = 'static';
        node = document.createElement('div');
        parent.appendChild(node);
        document.body.appendChild(parent);
    });

    afterEach(() => {
        parent.remove();
        vi.useRealTimers();
    });

    it('inserts a decorative track + thumb as a sibling of the scroller', () => {
        const handle = customScrollbar(node);

        const track = parent.querySelector('.etch-scrollbar-track');
        expect(track).not.toBeNull();
        expect(track?.getAttribute('aria-hidden')).toBe('true');
        expect(track?.querySelector('.etch-scrollbar-thumb')).not.toBeNull();
        // Track is a sibling that follows the scroller, so it can't affect scrollHeight.
        expect(node.nextElementSibling).toBe(track);

        handle.destroy();
    });

    it('promotes a statically positioned parent to relative for absolute anchoring', () => {
        const handle = customScrollbar(node);
        expect(parent.style.position).toBe('relative');
        handle.destroy();
    });

    it('restores everything it mutated on destroy', () => {
        const beforeScrollbarWidth = node.style.scrollbarWidth;
        const handle = customScrollbar(node);

        handle.destroy();

        expect(parent.querySelector('.etch-scrollbar-track')).toBeNull();
        expect(parent.style.position).toBe('static'); // restored, not left 'relative'
        expect(node.style.scrollbarWidth).toBe(beforeScrollbarWidth);
    });

    it('reveals on scroll and auto-hides after the delay', () => {
        vi.useFakeTimers();
        const handle = customScrollbar(node, { hideDelay: 1000 });
        const track = parent.querySelector('.etch-scrollbar-track')!;

        node.dispatchEvent(new Event('scroll'));
        expect(track.classList.contains('etch-visible')).toBe(true);

        vi.advanceTimersByTime(1000);
        expect(track.classList.contains('etch-visible')).toBe(false);

        handle.destroy();
    });
});
