import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { resetStores } from '$lib/stores/__tests__/helpers';
import { compactChat } from '$lib/stores/layout';
import MessageGroup from '../MessageGroup.svelte';
import type { ChatMessage, SenderProfile } from '$lib/types';

vi.mock('$lib/markdown', () => ({
    markdownToHtml: vi.fn((md: string) => `<p>${md}</p>`),
}));

vi.mock('$lib/highlight', () => ({
    hljs: { highlightElement: vi.fn() },
}));

// jsdom has no ResizeObserver; provide a no-op mock.
vi.stubGlobal('ResizeObserver', class {
    constructor(_cb: ResizeObserverCallback) {}
    observe() {}
    disconnect() {}
    unobserve() {}
});

// Capture requestAnimationFrame callbacks so tests can flush them after
// mocking scrollHeight on the rendered element.
let rafQueue: FrameRequestCallback[] = [];

beforeEach(() => {
    resetStores();
    rafQueue = [];
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
        rafQueue.push(cb);
        return rafQueue.length;
    });
});

afterEach(() => {
    vi.restoreAllMocks();
});

/** Flush all queued requestAnimationFrame callbacks and wait for Svelte to update. */
async function flushRAF() {
    const cbs = rafQueue.splice(0);
    cbs.forEach((cb) => cb(performance.now()));
    await tick();
}

function makeMsg(overrides: Partial<ChatMessage> = {}): ChatMessage {
    return {
        id: '$msg1',
        sender: '@alice:etch.gg',
        body: 'Hello world',
        html_body: null,
        media: null,
        timestamp: Date.now(),
        reactions: {},
        ...overrides,
    };
}

const sender: SenderProfile = { display_name: 'Alice', avatar_url: null };

describe('MessageGroup collapse', () => {
    it('short message shows no collapse button', async () => {
        render(MessageGroup, { props: { msg: makeMsg(), sender, continuation: false } });
        await flushRAF();

        expect(screen.queryByText('See more')).not.toBeInTheDocument();
    });

    it('long message shows "See more" button', async () => {
        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: false },
        });

        const bodyEl = container.querySelector('.body')!;
        Object.defineProperty(bodyEl, 'scrollHeight', { value: 500, configurable: true });

        await flushRAF();

        expect(screen.getByText('See more')).toBeInTheDocument();
    });

    it('long message wrapper has collapsed class', async () => {
        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: false },
        });

        const bodyEl = container.querySelector('.body')!;
        Object.defineProperty(bodyEl, 'scrollHeight', { value: 500, configurable: true });

        await flushRAF();

        expect(container.querySelector('.body-wrapper')).toHaveClass('collapsed');
    });

    it('"See more" expands the message', async () => {
        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: false },
        });

        const bodyEl = container.querySelector('.body')!;
        Object.defineProperty(bodyEl, 'scrollHeight', { value: 500, configurable: true });

        await flushRAF();

        await fireEvent.click(screen.getByText('See more'));

        expect(screen.getByText('See less')).toBeInTheDocument();
        expect(container.querySelector('.body-wrapper')).not.toHaveClass('collapsed');
    });

    it('"See less" collapses the message back', async () => {
        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: false },
        });

        const bodyEl = container.querySelector('.body')!;
        Object.defineProperty(bodyEl, 'scrollHeight', { value: 500, configurable: true });

        await flushRAF();

        // Expand
        await fireEvent.click(screen.getByText('See more'));
        // Collapse back
        await fireEvent.click(screen.getByText('See less'));

        expect(screen.getByText('See more')).toBeInTheDocument();
        expect(container.querySelector('.body-wrapper')).toHaveClass('collapsed');
    });

    it('message at exact threshold does not collapse', async () => {
        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: false },
        });

        const bodyEl = container.querySelector('.body')!;
        Object.defineProperty(bodyEl, 'scrollHeight', { value: 300, configurable: true });

        await flushRAF();

        expect(screen.queryByText('See more')).not.toBeInTheDocument();
    });
});

describe('MessageGroup compact mode', () => {
    it('default mode uses full-size avatar', () => {
        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: false },
        });

        expect(container.querySelector('.avatar')).toBeInTheDocument();
        expect(container.querySelector('.compact-avatar')).not.toBeInTheDocument();
    });

    it('compact mode uses small avatar', async () => {
        compactChat.set(true);
        await tick();

        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: false },
        });

        expect(container.querySelector('.compact-avatar')).toBeInTheDocument();
        expect(container.querySelector('.avatar')).not.toBeInTheDocument();
    });

    it('default continuation uses avatar-gutter', () => {
        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: true },
        });

        expect(container.querySelector('.avatar-gutter')).toBeInTheDocument();
        expect(container.querySelector('.compact-gutter')).not.toBeInTheDocument();
    });

    it('compact continuation uses compact-gutter', async () => {
        compactChat.set(true);
        await tick();

        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: true },
        });

        expect(container.querySelector('.compact-gutter')).toBeInTheDocument();
        expect(container.querySelector('.avatar-gutter')).not.toBeInTheDocument();
    });

    it('compact mode adds compact class to message-block', async () => {
        compactChat.set(true);
        await tick();

        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: false },
        });

        expect(container.querySelector('.message-block')).toHaveClass('compact');
    });

    it('default mode does not have compact class', () => {
        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: false },
        });

        expect(container.querySelector('.message-block')).not.toHaveClass('compact');
    });

    it('reacts when compactChat toggles after mount', async () => {
        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender, continuation: false },
        });

        expect(container.querySelector('.avatar')).toBeInTheDocument();
        expect(container.querySelector('.message-block')).not.toHaveClass('compact');

        compactChat.set(true);
        await tick();

        expect(container.querySelector('.compact-avatar')).toBeInTheDocument();
        expect(container.querySelector('.avatar')).not.toBeInTheDocument();
        expect(container.querySelector('.message-block')).toHaveClass('compact');
    });

    it('default mode passes full-size props to AvatarFallback', () => {
        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender: { ...sender, avatar_url: null }, continuation: false },
        });

        const fallback = container.querySelector('.avatar-fallback') as HTMLElement;
        expect(fallback).toBeInTheDocument();
        expect(fallback.style.width).toBe('40px');
        expect(fallback.style.height).toBe('40px');
        expect(fallback.style.fontSize).toBe('16px');
    });

    it('compact mode passes small props to AvatarFallback', async () => {
        compactChat.set(true);
        await tick();

        const { container } = render(MessageGroup, {
            props: { msg: makeMsg(), sender: { ...sender, avatar_url: null }, continuation: false },
        });

        const fallback = container.querySelector('.avatar-fallback') as HTMLElement;
        expect(fallback).toBeInTheDocument();
        expect(fallback.style.width).toBe('20px');
        expect(fallback.style.height).toBe('20px');
        expect(fallback.style.fontSize).toBe('10px');
    });
});
