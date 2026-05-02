import { writable } from 'svelte/store';
import { sendCoreCommand } from '$lib/ipc';

export type TransmissionMode = 'voice_activation' | 'continuous' | 'push_to_talk';

export const transmissionMode = writable<TransmissionMode>('voice_activation');
export const vadThreshold = writable<number>(60);
export const voiceHold = writable<number>(250);
export const useMumbleSettings = writable<boolean>(false);
export const deafenSuppressesNotifs = writable<boolean>(true);

export function setTransmissionMode(mode: TransmissionMode): void {
    transmissionMode.set(mode);
    sendCoreCommand({
        type: 'Mumble',
        data: { type: 'SetTransmissionMode', data: mode }
    });
}

export function setVadThreshold(value: number): void {
    vadThreshold.set(value);
    sendCoreCommand({
        type: 'Mumble',
        data: { type: 'SetVadThreshold', data: value / 100 }
    });
}

export function setVoiceHold(ms: number): void {
    voiceHold.set(ms);
    sendCoreCommand({
        type: 'Mumble',
        data: { type: 'SetVoiceHold', data: ms }
    });
}

export function setUseMumbleSettings(value: boolean): void {
    useMumbleSettings.set(value);
    sendCoreCommand({
        type: 'Mumble',
        data: { type: 'SetUseMumbleSettings', data: value }
    });
}

export function setDeafenSuppressesNotifs(value: boolean): void {
    deafenSuppressesNotifs.set(value);
    sendCoreCommand({
        type: 'System',
        data: { type: 'SetDeafenSuppressesNotifs', data: value }
    });
}
