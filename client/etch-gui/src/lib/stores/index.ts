import { initEventRouter } from './eventRouter';
import { initChannels } from './channels';

export { appFocused } from './eventRouter';

export { activeChannelId } from './activeChannel';
export { activeWindow, setActiveChannel, loadOlder, sendMessage, createDirectMessage, toggleReaction } from './messages';
export { channels, activeChannel, hideDm } from './channels';
export { currentUser } from './user';
export { isMuted, isDeafened, toggleMute, toggleDeafen } from './audio';
export { activeOverlay, overlayImageUrl, settingsTab, showRoomIds, openSettings, openImage, openConnect, closeOverlay } from './overlay';
export { serverBookmarks, selectedBookmarkId, connectingBookmark, passwordRequested, matrixConnecting, mediaBaseUrl, loadSettings, addBookmark, updateBookmark, removeBookmark, connectToServer } from './servers';
export { replyingTo, setReply, clearReply } from './compose';
export { voiceChannels, voiceUsers, voiceConnected, mumbleStatus, usersByChannel, talkingUsers } from './voiceState';
export type { VoiceChannel, VoiceUser, MumbleStatus } from './voiceState';
export { errorLog, toastError, showToast } from './errors';
export type { ErrorEntry } from './errors';
export { userVolumes, setUserVolume } from './userVolumes';
export { sfxVolume, playSfx, setSfxDeafened } from './sfx';
export type { SfxName } from './sfx';
export { transmissionMode, setTransmissionMode, vadThreshold, setVadThreshold, voiceHold, setVoiceHold, useMumbleSettings, setUseMumbleSettings } from './voiceSettings';
export type { TransmissionMode } from './voiceSettings';
export { theme, initTheme } from './theme';
export { sidebarCollapsed, sidebarPeeking, sidebarVisuallyCollapsed, sidebarContentCollapsed, sidebarTransitioning, toggleSidebar, setPeeking, startPeekClose, cancelPeekClose, destroySidebar } from './sidebar';
export type { Theme } from './theme';
export { updateStatus, updateVersion, updateError, checkForUpdate, restartApp } from './updater';
export type { UpdateStatus } from './updater';

export function initStores(): void {
    initEventRouter();
    initChannels();
}
