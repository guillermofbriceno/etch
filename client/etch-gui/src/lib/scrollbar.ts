export interface ScrollbarOptions {
    /** Minimum thumb height in px. Default: 30 */
    minThumbHeight?: number;
    /** Auto-hide delay in ms after scroll/hover stops. Default: 1200 */
    hideDelay?: number;
    /** Width of the scrollbar in px. Default: 8 */
    width?: number;
    /** Padding from the top/bottom edge of the track. Default: 2 */
    trackPadding?: number;
    /** Inset from the right edge of the container in px. Default: 6 */
    rightInset?: number;
}

const DEFAULTS: Required<ScrollbarOptions> = {
    minThumbHeight: 30,
    hideDelay: 1200,
    width: 8,
    trackPadding: 2,
    rightInset: 6,
};

/** Geometry needed to render and drive the thumb. Independent of any DOM. */
export interface ThumbGeometry {
    /** Usable track length (the track minus top + bottom padding). */
    trackHeight: number;
    /** Rendered thumb height, after the minimum clamp. */
    thumbHeight: number;
    /** How far the thumb's top can travel within the track. */
    thumbRange: number;
    /** Maximum scrollTop of the content. */
    maxScrollTop: number;
}

const clamp01 = (n: number): number => Math.min(Math.max(n, 0), 1);

/**
 * Compute thumb sizing from the scroller's dimensions. Pure: the single source
 * of truth for the proportional-sizing math shared by render, drag, and click.
 */
export function thumbGeometry(
    scrollHeight: number,
    clientHeight: number,
    opts: Pick<Required<ScrollbarOptions>, 'trackPadding' | 'minThumbHeight'>,
): ThumbGeometry {
    const trackHeight = Math.max(clientHeight - opts.trackPadding * 2, 0);
    const ratio = scrollHeight > 0 ? clientHeight / scrollHeight : 1;
    // Clamp to the minimum for usability, but never let it exceed the track.
    const thumbHeight = Math.min(Math.max(ratio * trackHeight, opts.minThumbHeight), trackHeight);
    return {
        trackHeight,
        thumbHeight,
        thumbRange: Math.max(trackHeight - thumbHeight, 0),
        maxScrollTop: Math.max(scrollHeight - clientHeight, 0),
    };
}

/** Thumb top offset (including the track padding) for a given scroll position. */
export function thumbTopForScroll(scrollTop: number, geom: ThumbGeometry, trackPadding: number): number {
    const fraction = geom.maxScrollTop > 0 ? clamp01(scrollTop / geom.maxScrollTop) : 0;
    return trackPadding + fraction * geom.thumbRange;
}

/** Scroll position for a drag that moved the thumb `deltaY` px from `startScrollTop`. */
export function scrollTopForDrag(startScrollTop: number, deltaY: number, geom: ThumbGeometry): number {
    if (geom.thumbRange <= 0) return startScrollTop;
    return startScrollTop + (deltaY / geom.thumbRange) * geom.maxScrollTop;
}

/** Scroll position for a track click at `clickY`, measured from the padded track top. */
export function scrollTopForTrackClick(clickY: number, geom: ThumbGeometry): number {
    if (geom.trackHeight <= 0) return 0;
    return clamp01(clickY / geom.trackHeight) * geom.maxScrollTop;
}

let stylesInjected = false;

function injectStyles(): void {
    if (stylesInjected) return;
    stylesInjected = true;

    const style = document.createElement('style');
    style.id = 'etch-scrollbar-styles';
    style.textContent = `
        .etch-scrollbar-track {
            position: absolute;
            top: 0;
            /* Pass clicks through to content while hidden, capture while visible.
               The thumb inherits this, so an invisible bar never steals clicks. */
            pointer-events: none;
            z-index: 10;
            opacity: 0;
            transition: opacity 0.3s ease;
            box-sizing: border-box;
        }

        .etch-scrollbar-track.etch-visible {
            opacity: 1;
            pointer-events: auto;
        }

        .etch-scrollbar-thumb {
            position: absolute;
            right: 0;
            border-radius: 4px;
            background-color: var(--scrollbar-thumb);
            pointer-events: inherit;
            transition: background-color 0.15s ease;
            cursor: default;
        }

        .etch-scrollbar-thumb:hover {
            background-color: color-mix(in srgb, var(--scrollbar-thumb) 70%, white);
        }

        .etch-scrollbar-track.etch-dragging .etch-scrollbar-thumb {
            background-color: color-mix(in srgb, var(--scrollbar-thumb) 50%, white);
            transition: none;
        }
    `;
    document.head.appendChild(style);
}

export function customScrollbar(
    node: HTMLElement,
    options?: ScrollbarOptions,
): { update: (opts?: ScrollbarOptions) => void; destroy: () => void } {
    injectStyles();

    let opts = { ...DEFAULTS, ...options };
    let hideTimer: ReturnType<typeof setTimeout> | undefined;
    let isDragging = false;
    let dragStartY = 0;
    let dragStartScrollTop = 0;

    // Hide native scrollbar (remember the prior inline value to restore on destroy)
    const prevScrollbarWidth = node.style.scrollbarWidth;
    node.style.scrollbarWidth = 'none';

    // --- Place track as a sibling, outside the scroll container ---
    // This guarantees the track can never affect scrollHeight.
    const parent = node.parentElement!;
    let prevParentPosition: string | undefined;
    if (getComputedStyle(parent).position === 'static') {
        prevParentPosition = parent.style.position;
        parent.style.position = 'relative';
    }

    const track = document.createElement('div');
    track.className = 'etch-scrollbar-track';
    track.setAttribute('aria-hidden', 'true');
    track.style.width = `${opts.width}px`;
    track.style.right = `${opts.rightInset}px`;

    const thumb = document.createElement('div');
    thumb.className = 'etch-scrollbar-thumb';
    thumb.style.width = `${opts.width}px`;
    track.appendChild(thumb);

    // Insert track immediately after the scroller in the parent
    node.insertAdjacentElement('afterend', track);

    // --- Sync track position/size to the scroller's bounding box ---

    function syncTrackToNode(): void {
        const nodeRect = node.getBoundingClientRect();
        const parentRect = parent.getBoundingClientRect();

        track.style.top = `${nodeRect.top - parentRect.top}px`;
        track.style.height = `${nodeRect.height}px`;
    }

    // --- Update thumb position and size ---

    function update(): void {
        const { scrollTop, scrollHeight, clientHeight } = node;

        if (scrollHeight <= clientHeight || clientHeight === 0) {
            track.style.display = 'none';
            return;
        }
        track.style.display = '';

        syncTrackToNode();

        const geom = thumbGeometry(scrollHeight, clientHeight, opts);
        thumb.style.height = `${geom.thumbHeight}px`;
        thumb.style.top = `${thumbTopForScroll(scrollTop, geom, opts.trackPadding)}px`;
    }

    // Coalesce reactive updates (scroll, resize, DOM mutations) into one
    // measure-and-write per frame to avoid layout thrash on busy containers.
    let rafId: number | undefined;
    function scheduleUpdate(): void {
        if (rafId !== undefined) return;
        rafId = requestAnimationFrame(() => {
            rafId = undefined;
            update();
        });
    }

    // --- Visibility ---

    function showScrollbar(): void {
        clearTimeout(hideTimer);
        track.classList.add('etch-visible');
    }

    function scheduleHide(): void {
        clearTimeout(hideTimer);
        hideTimer = setTimeout(() => {
            if (!isDragging) {
                track.classList.remove('etch-visible');
            }
        }, opts.hideDelay);
    }

    // --- Scroll listener ---

    function onScroll(): void {
        scheduleUpdate();
        showScrollbar();
        scheduleHide();
    }

    // --- Drag handling ---

    function onThumbMouseDown(e: MouseEvent): void {
        if (e.button !== 0) return;
        e.preventDefault();
        e.stopPropagation();

        isDragging = true;
        dragStartY = e.clientY;
        dragStartScrollTop = node.scrollTop;
        track.classList.add('etch-dragging');
        document.body.style.userSelect = 'none';

        window.addEventListener('mousemove', onWindowMouseMove);
        window.addEventListener('mouseup', onWindowMouseUp);
    }

    function onWindowMouseMove(e: MouseEvent): void {
        const geom = thumbGeometry(node.scrollHeight, node.clientHeight, opts);
        if (geom.thumbRange <= 0) return;
        node.scrollTop = scrollTopForDrag(dragStartScrollTop, e.clientY - dragStartY, geom);
    }

    function onWindowMouseUp(): void {
        isDragging = false;
        track.classList.remove('etch-dragging');
        document.body.style.userSelect = '';

        window.removeEventListener('mousemove', onWindowMouseMove);
        window.removeEventListener('mouseup', onWindowMouseUp);

        scheduleHide();
    }

    // --- Track click ---

    function onTrackMouseDown(e: MouseEvent): void {
        if (e.target === thumb || e.button !== 0) return;
        e.preventDefault();

        const trackRect = track.getBoundingClientRect();
        const clickY = e.clientY - trackRect.top - opts.trackPadding;
        const geom = thumbGeometry(node.scrollHeight, node.clientHeight, opts);

        if (geom.trackHeight <= 0) return;

        node.scrollTop = scrollTopForTrackClick(clickY, geom);

        // Initiate drag so click-and-drag works in one motion
        isDragging = true;
        dragStartY = e.clientY;
        dragStartScrollTop = node.scrollTop;
        track.classList.add('etch-dragging');
        document.body.style.userSelect = 'none';

        window.addEventListener('mousemove', onWindowMouseMove);
        window.addEventListener('mouseup', onWindowMouseUp);
    }

    // --- Hover visibility ---

    function onTrackMouseEnter(): void {
        showScrollbar();
    }

    function onTrackMouseLeave(): void {
        if (!isDragging) scheduleHide();
    }

    // --- Observers ---

    const resizeObs = new ResizeObserver(() => scheduleUpdate());
    resizeObs.observe(node);

    const mutationObs = new MutationObserver(() => scheduleUpdate());
    mutationObs.observe(node, { childList: true, subtree: true });

    // --- Wire up ---

    node.addEventListener('scroll', onScroll, { passive: true });
    thumb.addEventListener('mousedown', onThumbMouseDown);
    track.addEventListener('mousedown', onTrackMouseDown);
    track.addEventListener('mouseenter', onTrackMouseEnter);
    track.addEventListener('mouseleave', onTrackMouseLeave);

    // Initial render
    update();

    return {
        update(newOpts?: ScrollbarOptions) {
            opts = { ...DEFAULTS, ...newOpts };
            track.style.width = `${opts.width}px`;
            track.style.right = `${opts.rightInset}px`;
            thumb.style.width = `${opts.width}px`;
            update();
        },
        destroy() {
            clearTimeout(hideTimer);
            if (rafId !== undefined) cancelAnimationFrame(rafId);
            node.removeEventListener('scroll', onScroll);
            thumb.removeEventListener('mousedown', onThumbMouseDown);
            track.removeEventListener('mousedown', onTrackMouseDown);
            track.removeEventListener('mouseenter', onTrackMouseEnter);
            track.removeEventListener('mouseleave', onTrackMouseLeave);
            window.removeEventListener('mousemove', onWindowMouseMove);
            window.removeEventListener('mouseup', onWindowMouseUp);
            resizeObs.disconnect();
            mutationObs.disconnect();
            track.remove();

            // Restore styles we mutated on elements the action doesn't own.
            node.style.scrollbarWidth = prevScrollbarWidth;
            if (prevParentPosition !== undefined) {
                parent.style.position = prevParentPosition;
            }
            // Release the global selection lock if we were torn down mid-drag.
            if (isDragging) document.body.style.userSelect = '';
        },
    };
}
