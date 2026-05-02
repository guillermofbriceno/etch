import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { get } from 'svelte/store';
import { resetStores } from '$lib/stores/__tests__/helpers';
import { compactChat } from '$lib/stores/layout';
import SettingsAppearance from '../settings/SettingsAppearance.svelte';

beforeEach(() => {
    resetStores();
});

afterEach(() => {
    vi.restoreAllMocks();
});

describe('SettingsAppearance compact chat toggle', () => {
    it('checkbox is unchecked when compactChat is false', () => {
        const { container } = render(SettingsAppearance);
        const checkbox = container.querySelector('input[type="checkbox"]') as HTMLInputElement;

        expect(checkbox).toBeInTheDocument();
        expect(checkbox.checked).toBe(false);
    });

    it('checkbox is checked when compactChat is true', async () => {
        compactChat.set(true);
        await tick();

        const { container } = render(SettingsAppearance);
        const checkbox = container.querySelector('input[type="checkbox"]') as HTMLInputElement;

        expect(checkbox.checked).toBe(true);
    });

    it('clicking checkbox toggles compactChat store to true', async () => {
        const { container } = render(SettingsAppearance);
        const checkbox = container.querySelector('input[type="checkbox"]') as HTMLInputElement;

        await fireEvent.click(checkbox);

        expect(get(compactChat)).toBe(true);
    });

    it('clicking checkbox twice returns compactChat store to false', async () => {
        const { container } = render(SettingsAppearance);
        const checkbox = container.querySelector('input[type="checkbox"]') as HTMLInputElement;

        await fireEvent.click(checkbox);
        await fireEvent.click(checkbox);

        expect(get(compactChat)).toBe(false);
    });

    it('checkbox reflects external store changes', async () => {
        const { container } = render(SettingsAppearance);
        const checkbox = container.querySelector('input[type="checkbox"]') as HTMLInputElement;

        expect(checkbox.checked).toBe(false);

        compactChat.set(true);
        await tick();

        expect(checkbox.checked).toBe(true);
    });
});
