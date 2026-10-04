import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { resetStores } from '$lib/stores/__tests__/helpers';
import { currentUser } from '$lib/stores/user';
import { nameColors, canSetNameColor, handleMatrixEvent } from '$lib/stores/nameColors';
import { hexHue, hueColor } from '$lib/userColor';
import type { CoreCommand, MatrixCommand } from '$lib/ipc';
import type { NameColor } from '$lib/types';
import SettingsAccount from '../settings/SettingsAccount.svelte';

const ME = '@alice:example.org';

beforeEach(() => {
    resetStores();
    vi.mocked(invoke).mockClear();
    currentUser.set({ username: 'alice', matrixId: ME, displayName: 'Alice', avatarUrl: null });
    canSetNameColor.set(true);
});

function storeMine(color: NameColor | null): void {
    nameColors.set(new Map([[ME, color]]));
}

function section(): HTMLElement {
    return screen.getByRole('group', { name: 'Name Color' });
}

function slider(): HTMLInputElement {
    return within(section()).getByRole('slider');
}

function hexField(): HTMLInputElement {
    return within(section()).getByRole('textbox', { name: 'Name Color' });
}

function type(value: string): Promise<boolean> {
    return fireEvent.input(hexField(), { target: { value } });
}

function tooDarkMessage(): HTMLElement | null {
    return within(section()).queryByText('Too dark to read on the chat background.');
}

function automatic(): HTMLInputElement {
    return within(section()).getByRole('checkbox', { name: 'Automatic' });
}

function applyButton(): HTMLButtonElement {
    return within(section()).getByRole('button', { name: 'Apply' });
}

function sent(type: MatrixCommand['type']): CoreCommand[] {
    return vi.mocked(invoke).mock.calls
        .map(([, args]) => (args as { command: CoreCommand }).command)
        .filter((command) => command.type === 'Matrix' && command.data.type === type);
}

function setNameColor(color: NameColor | null): CoreCommand {
    return { type: 'Matrix', data: { type: 'SetNameColor', data: color } };
}

describe('SettingsAccount name color', () => {
    it('is hidden when the server does not let the user set one', () => {
        canSetNameColor.set(false);
        render(SettingsAccount);

        expect(screen.queryByRole('group', { name: 'Name Color' })).not.toBeInTheDocument();
    });

    it('follows the stored color when it arrives after opening', async () => {
        render(SettingsAccount);
        expect(automatic().checked).toBe(true);

        handleMatrixEvent({ type: 'NameColors', data: [{ user_id: ME, color: { color: '#ff8800' } }] });
        await tick();

        expect(hexField().value).toBe('#ff8800');
        expect(automatic().checked).toBe(false);
        expect(applyButton()).toBeDisabled();
    });

    it('sends a typed color on Apply as lowercase #rrggbb, leaving the store for core to update', async () => {
        storeMine(null);
        render(SettingsAccount);

        await type(' FF8800 ');
        await fireEvent.click(applyButton());

        expect(sent('SetNameColor')).toEqual([setNameColor({ color: '#ff8800' })]);
        expect(get(nameColors).get(ME)).toBeNull();
    });

    it('refuses a color it cannot use, and says why when it is too dark', async () => {
        storeMine(null);
        render(SettingsAccount);

        await type('#f80');
        expect(applyButton()).toBeDisabled();
        expect(tooDarkMessage()).not.toBeInTheDocument();

        await type('#7c7c7c');
        expect(applyButton()).toBeDisabled();
        expect(tooDarkMessage()).toBeInTheDocument();

        await type('#7d7d7d');
        expect(applyButton()).toBeEnabled();
        expect(tooDarkMessage()).not.toBeInTheDocument();
    });

    it('fills the field from the slider, leaving the slider where it was put', async () => {
        storeMine(null);
        render(SettingsAccount);
        expect(hexHue(hueColor(43)), 'a hue that does not survive the trip through hex').not.toBe(43);

        await fireEvent.input(slider(), { target: { value: '43' } });

        expect(hexField().value).toBe(hueColor(43));
        expect(slider().value).toBe('43');
        await fireEvent.click(applyButton());

        expect(sent('SetNameColor')).toEqual([setNameColor({ color: hueColor(43) })]);
    });

    it('sends null on Apply when Automatic is chosen', async () => {
        storeMine({ color: '#ff8800' });
        render(SettingsAccount);

        await fireEvent.click(automatic());
        await fireEvent.click(applyButton());

        expect(sent('SetNameColor')).toEqual([setNameColor(null)]);
    });
});
