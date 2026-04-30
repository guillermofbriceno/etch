import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { setTransmissionMode, setVadThreshold, setVoiceHold, setUseMumbleSettings, transmissionMode, vadThreshold, voiceHold, useMumbleSettings } from '../voiceSettings';
import { resetStores } from './helpers';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

beforeEach(() => {
    resetStores();
    vi.mocked(invoke).mockClear();
});

describe('voiceSettings IPC commands', () => {
    describe('setTransmissionMode', () => {
        it('sends Mumble > SetTransmissionMode', () => {
            setTransmissionMode('push_to_talk');

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'Mumble', data: { type: 'SetTransmissionMode', data: 'push_to_talk' } },
            });
        });

        it('updates the transmissionMode store', () => {
            setTransmissionMode('continuous');

            expect(get(transmissionMode)).toBe('continuous');
        });

        it('accepts all valid transmission modes', () => {
            for (const mode of ['voice_activation', 'continuous', 'push_to_talk'] as const) {
                setTransmissionMode(mode);
                expect(get(transmissionMode)).toBe(mode);
            }
        });
    });

    describe('setVadThreshold', () => {
        it('sends Mumble > SetVadThreshold (value / 100)', () => {
            setVadThreshold(75);

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'Mumble', data: { type: 'SetVadThreshold', data: 0.75 } },
            });
        });

        it('updates the vadThreshold store', () => {
            setVadThreshold(80);

            expect(get(vadThreshold)).toBe(80);
        });

        it('handles 0 (minimum)', () => {
            setVadThreshold(0);

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'Mumble', data: { type: 'SetVadThreshold', data: 0 } },
            });
            expect(get(vadThreshold)).toBe(0);
        });

        it('handles 100 (maximum)', () => {
            setVadThreshold(100);

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'Mumble', data: { type: 'SetVadThreshold', data: 1 } },
            });
            expect(get(vadThreshold)).toBe(100);
        });
    });

    describe('setVoiceHold', () => {
        it('sends Mumble > SetVoiceHold', () => {
            setVoiceHold(300);

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'Mumble', data: { type: 'SetVoiceHold', data: 300 } },
            });
        });

        it('updates the voiceHold store', () => {
            setVoiceHold(500);

            expect(get(voiceHold)).toBe(500);
        });

        it('handles 0 ms', () => {
            setVoiceHold(0);

            expect(get(voiceHold)).toBe(0);
            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'Mumble', data: { type: 'SetVoiceHold', data: 0 } },
            });
        });
    });

    describe('setUseMumbleSettings', () => {
        it('sends Mumble > SetUseMumbleSettings', () => {
            setUseMumbleSettings(true);

            expect(invoke).toHaveBeenCalledWith('core_command', {
                command: { type: 'Mumble', data: { type: 'SetUseMumbleSettings', data: true } },
            });
        });

        it('updates the useMumbleSettings store', () => {
            setUseMumbleSettings(true);
            expect(get(useMumbleSettings)).toBe(true);

            setUseMumbleSettings(false);
            expect(get(useMumbleSettings)).toBe(false);
        });
    });
});
