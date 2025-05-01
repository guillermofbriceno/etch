import { invoke } from '@tauri-apps/api/core';
import type { TimelineEntry, RoomInfo, ServerBookmark } from '$lib/types';

// --- Event envelope types (Rust → Frontend) ---

export type MatrixEvent =
    | { type: 'TimelineAppend'; data: [string, TimelineEntry[]] }
    | { type: 'TimelinePushBack'; data: [string, TimelineEntry] }
    | { type: 'TimelinePushFront'; data: [string, TimelineEntry] }
    | { type: 'TimelineInsert'; data: [string, number, TimelineEntry] }
    | { type: 'TimelineSet'; data: [string, number, TimelineEntry] }
    | { type: 'TimelineRemove'; data: [string, number] }
    | { type: 'TimelineCleared'; data: string }
    | { type: 'TimelineReset'; data: [string, TimelineEntry[]] }
    | { type: 'ChannelList'; data: RoomInfo[] }
    | { type: 'DmCreated'; data: RoomInfo }
    | { type: 'HomeserverResolved'; data: string }
    | { type: 'CurrentUser'; data: { username: string; matrix_id: string; display_name: string | null; avatar_url: string | null } }
    | { type: 'PasswordRequest' }
    | { type: 'PaginationComplete'; data: [string, boolean] }
    | { type: 'ConnectionState'; data: { type: 'Disconnected' } | { type: 'Connecting' } | { type: 'Connected' } | { type: 'Failed'; reason: string; retries: number; retry_in_secs: number } };

export type MumbleEvent =
    | { type: 'LocalSession'; data: number }
    | { type: 'UserState'; data: { session_id: number; name: string | null; display_name: string | null; avatar_url: string | null; channel_id: number | null; self_mute: boolean | null; self_deaf: boolean | null; hash: string | null } }
    | { type: 'UserRemoved'; data: number }
    | { type: 'UserTalking'; data: { session_id: number; talking: boolean } }
    | { type: 'UserVolume'; data: { session_id: number; volume_db: number } }
    | { type: 'ChannelState'; data: { id: number; name: string; parent: number } }
    | { type: 'ChannelRemoved'; data: number }
    | { type: 'TransmissionModeChanged'; data: 'voice_activation' | 'continuous' | 'push_to_talk' }
    | { type: 'VadThresholdChanged'; data: number }
    | { type: 'VoiceHoldChanged'; data: number }
    | { type: 'ConnectionState'; data: { type: 'Disconnected' } | { type: 'Connecting' } | { type: 'Connected' } | { type: 'Failed'; reason: string; retries: number; retry_in_secs: number } };

export type SystemEvent =
    | { type: 'ServerReset' }
    | { type: 'ConnectionLost' }
    | { type: 'SettingsLoaded'; data: { bookmarks: ServerBookmark[]; transmission_mode: string | null; vad_threshold: number | null; voice_hold: number | null; use_mumble_settings: boolean | null; hidden_dms: string[] } }
    | { type: 'LogError'; data: { message: string; target: string } }
    | { type: 'UserProfileChanged'; data: { username: string; display_name: string | null; avatar_url: string | null } };

export type CoreEvent =
    | { type: 'Matrix'; data: MatrixEvent }
    | { type: 'Mumble'; data: MumbleEvent }
    | { type: 'System'; data: SystemEvent };

// --- Command envelope types (Frontend → Rust) ---

export type MatrixCommand =
    | { type: 'SendMessage'; data: { room_id: string; text: string; html_body: string | null; attachment_path: string | null } }
    | { type: 'ToggleReaction'; data: { room_id: string; event_id: string; key: string } }
    | { type: 'CreateDirectMessage'; data: { target_user_id: string } }
    | { type: 'SetDisplayName'; data: string }
    | { type: 'SetAvatar'; data: string }
    | { type: 'ChangePassword'; data: { current_password: string; new_password: string } }
    | { type: 'SendReadReceipt'; data: { room_id: string; event_id: string } }
    | { type: 'PaginateBackwards'; data: { room_id: string } }
    | { type: 'EnableEncryption'; data: { room_id: string } };

export type MumbleCommand =
    | { type: 'SwitchChannel'; data: number }
    | { type: 'MuteSelf'; data: boolean }
    | { type: 'DeafenSelf'; data: boolean }
    | { type: 'SetUserVolume'; data: { session_id: number; volume_db: number } }
    | { type: 'SetTransmissionMode'; data: 'voice_activation' | 'continuous' | 'push_to_talk' }
    | { type: 'SetVadThreshold'; data: number }
    | { type: 'SetVoiceHold'; data: number }
    | { type: 'SetUseMumbleSettings'; data: boolean };

export type SystemCommand =
    | { type: 'ConnectToServer'; data: { username: string; hostname: string; port: string; password: string | null; mumble_host: string | null; mumble_port: number | null; mumble_username: string | null; mumble_password: string | null } }
    | { type: 'LoadSettings' }
    | { type: 'SaveBookmarks'; data: ServerBookmark[] }
    | { type: 'MuteMic'; data: boolean }
    | { type: 'Deafen'; data: boolean }
    | { type: 'OpenMumbleGui'; data: string }
    | { type: 'RestartMumble'; data: string }
    | { type: 'SetLogLevel'; data: string }
    | { type: 'TestError' }
    | { type: 'HideDm'; data: { room_id: string } }
    | { type: 'UnhideDm'; data: { room_id: string } };

export type CoreCommand =
    | { type: 'Matrix'; data: MatrixCommand }
    | { type: 'Mumble'; data: MumbleCommand }
    | { type: 'System'; data: SystemCommand };

// --- Command helper ---

export async function sendCoreCommand(command: CoreCommand): Promise<void> {
    await invoke('core_command', { command });
}
