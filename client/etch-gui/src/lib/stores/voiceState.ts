import { writable, derived, get } from 'svelte/store';
import type { MumbleEvent, SystemEvent } from '$lib/ipc';
import { playSfx } from './sfx';
import { isMuted, isDeafened } from './audio';
import { userVolumes } from './userVolumes';
import { transmissionMode, vadThreshold, voiceHold, useMumbleSettings } from './voiceSettings';
import type { TransmissionMode } from './voiceSettings';

export type VoiceChannel = {
    id: number;
    name: string;
    parent: number;
};

export type VoiceUser = {
    session_id: number;
    name: string;
    display_name: string | null;
    avatar_url: string | null;
    channel_id: number;
    muted: boolean;
    deafened: boolean;
    talking: boolean;
    hash: string | null;
};

export const voiceChannels = writable<Map<number, VoiceChannel>>(new Map());
export const voiceUsers = writable<Map<number, VoiceUser>>(new Map());
export type MumbleStatus = 'disconnected' | 'connecting' | 'connected';
export const mumbleStatus = writable<MumbleStatus>('disconnected');
export const voiceConnected = derived(mumbleStatus, ($s) => $s === 'connected');

// Users grouped by channel ID, sorted alphabetically by name
export const usersByChannel = derived(voiceUsers, ($users) => {
    const grouped = new Map<number, VoiceUser[]>();
    for (const user of $users.values()) {
        const list = grouped.get(user.channel_id) ?? [];
        list.push(user);
        grouped.set(user.channel_id, list);
    }
    for (const list of grouped.values()) {
        list.sort((a, b) => a.name.localeCompare(b.name));
    }
    return grouped;
});

let connected = false;
let settled = false;
let localSession: number | null = null;

// Handler called by eventRouter for Mumble events
export function handleMumbleEvent(me: MumbleEvent): void {
    switch (me.type) {
        case 'LocalSession': {
            localSession = me.data;
            break;
        }
        case 'ChannelState': {
            const ch = me.data;
            voiceChannels.update((m) => {
                m.set(ch.id, { id: ch.id, name: ch.name, parent: ch.parent });
                return new Map(m);
            });
            break;
        }
        case 'ChannelRemoved': {
            voiceChannels.update((m) => {
                m.delete(me.data);
                return new Map(m);
            });
            break;
        }
        case 'UserState': {
            const u = me.data;
            voiceUsers.update((m) => {
                const existing = m.get(u.session_id);
                if (settled && u.session_id !== localSession) {
                    const localUser = localSession != null ? m.get(localSession) : null;
                    if (localUser) {
                        const oldCh = existing?.channel_id;
                        const newCh = u.channel_id ?? oldCh;
                        if (!existing && newCh === localUser.channel_id) {
                            playSfx('user_join');
                        } else if (existing && newCh !== oldCh) {
                            if (newCh === localUser.channel_id) playSfx('user_join');
                            else if (oldCh === localUser.channel_id) playSfx('user_leave');
                        }
                    }
                }
                m.set(u.session_id, {
                    session_id: u.session_id,
                    name: u.name ?? existing?.name ?? '',
                    display_name: u.display_name ?? existing?.display_name ?? null,
                    avatar_url: u.avatar_url ?? existing?.avatar_url ?? null,
                    channel_id: u.channel_id ?? existing?.channel_id ?? 0,
                    muted: u.self_mute ?? existing?.muted ?? false,
                    deafened: u.self_deaf ?? existing?.deafened ?? false,
                    talking: existing?.talking ?? false,
                    hash: u.hash ?? existing?.hash ?? null,
                });
                return new Map(m);
            });
            // Sync local mute/deafen UI state
            if (u.session_id === localSession) {
                if (u.self_mute != null) isMuted.set(u.self_mute);
                if (u.self_deaf != null) isDeafened.set(u.self_deaf);
            }
            break;
        }
        case 'UserTalking': {
            const { session_id, talking } = me.data;
            voiceUsers.update((m) => {
                const existing = m.get(session_id);
                if (existing) {
                    m.set(session_id, { ...existing, talking });
                }
                return new Map(m);
            });
            break;
        }
        case 'UserVolume': {
            const { session_id, volume_db } = me.data;
            userVolumes.update(v => ({ ...v, [session_id]: Math.round(volume_db * 10) / 10 }));
            break;
        }
        case 'UserRemoved': {
            voiceUsers.update((m) => {
                const removed = m.get(me.data);
                const localUser = localSession != null ? m.get(localSession) : null;
                m.delete(me.data);
                if (removed && localUser && removed.channel_id === localUser.channel_id) {
                    playSfx('user_leave');
                }
                return new Map(m);
            });
            break;
        }
        case 'TransmissionModeChanged': {
            if (get(useMumbleSettings)) transmissionMode.set(me.data as TransmissionMode);
            break;
        }
        case 'VadThresholdChanged': {
            if (get(useMumbleSettings)) vadThreshold.set(Math.round(me.data * 100));
            break;
        }
        case 'VoiceHoldChanged': {
            if (get(useMumbleSettings)) voiceHold.set(me.data);
            break;
        }
        case 'ConnectionState': {
            if (me.data.type === 'Connected') {
                connected = true;
                settled = false;
                mumbleStatus.set('connected');
                playSfx('server_join');
                setTimeout(() => { settled = true; }, 3000);
            } else if (me.data.type === 'Connecting') {
                mumbleStatus.set('connecting');
            } else if (me.data.type === 'Disconnected') {
                connected = false;
                settled = false;
                localSession = null;
                mumbleStatus.set('disconnected');
                voiceChannels.set(new Map());
                voiceUsers.set(new Map());
                playSfx('server_disconnect');
            }
            break;
        }
    }
}

export function handleSystemEvent(se: SystemEvent): void {
    if (se.type !== 'UserProfileChanged') return;
    const { username, display_name, avatar_url } = se.data;
    voiceUsers.update((m) => {
        let changed = false;
        for (const [id, user] of m) {
            if (user.name === username) {
                m.set(id, { ...user, display_name, avatar_url });
                changed = true;
            }
        }
        return changed ? new Map(m) : m;
    });
}
