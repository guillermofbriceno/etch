import { vi } from 'vitest';
import '@testing-library/jest-dom/vitest';

// Mock all Tauri API modules so tests run in jsdom without a real Tauri runtime.
// vi.mock() calls are hoisted by Vitest and apply before any test module imports.

vi.mock('@tauri-apps/api/core', () => ({
    invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn().mockResolvedValue(vi.fn()),
    emit: vi.fn(),
}));

vi.mock('@tauri-apps/api/window', () => ({
    getCurrentWindow: vi.fn().mockReturnValue({
        label: 'main',
        listen: vi.fn(),
        onCloseRequested: vi.fn(),
        startDragging: vi.fn(),
        minimize: vi.fn(),
        toggleMaximize: vi.fn(),
        close: vi.fn(),
    }),
}));

vi.mock('@tauri-apps/api/app', () => ({
    getVersion: vi.fn().mockResolvedValue('0.0.0-test'),
}));

vi.mock('@tauri-apps/api/path', () => ({
    tempDir: vi.fn().mockResolvedValue('/tmp'),
    join: vi.fn((...parts: string[]) => Promise.resolve(parts.join('/'))),
}));

vi.mock('@tauri-apps/plugin-dialog', () => ({
    open: vi.fn(),
    save: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-fs', () => ({
    writeFile: vi.fn(),
    remove: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-opener', () => ({
    openUrl: vi.fn(),
    openPath: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-process', () => ({
    relaunch: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-store', () => ({
    load: vi.fn().mockResolvedValue({
        get: vi.fn(),
        set: vi.fn(),
        save: vi.fn(),
    }),
}));
