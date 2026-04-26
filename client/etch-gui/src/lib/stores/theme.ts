import { writable } from 'svelte/store';

export type Theme = 'default' | 'oled';

const STORAGE_KEY = 'etch-theme';

function loadTheme(): Theme {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === 'oled') return 'oled';
    return 'default';
}

function applyTheme(t: Theme): void {
    if (t === 'default') {
        document.documentElement.removeAttribute('data-theme');
    } else {
        document.documentElement.setAttribute('data-theme', t);
    }
}

export const theme = writable<Theme>(loadTheme());

export function initTheme(): void {
    applyTheme(loadTheme());
    theme.subscribe((t) => {
        localStorage.setItem(STORAGE_KEY, t);
        applyTheme(t);
    });
}
