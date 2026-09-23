import { writable } from 'svelte/store';
import { declareStores } from './session';

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

// Device-scoped. An appearance preference, persisted to localStorage.
export const theme = writable<Theme>(loadTheme());

declareStores('device', 'theme');

export function initTheme(): void {
    applyTheme(loadTheme());
    theme.subscribe((t) => {
        localStorage.setItem(STORAGE_KEY, t);
        applyTheme(t);
    });
}