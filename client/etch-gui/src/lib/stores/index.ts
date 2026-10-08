import { initEventRouter } from './eventRouter';
import { initChannels } from './channels';

export { appFocused } from './eventRouter';

export { activeChannelId } from './activeChannel';
export { activeWindow, setActiveChannel, loadOlder, sendMessage, sendAttachment, editMessage, redactMessage, createDirectMessage, toggleReaction } from './messages';
export { channels, activeChannel, hideDm, dmLastActivity } from './channels';
export { currentUser } from './user';
export { nameColors, canSetNameColor, colorOf, chosenColorOf } from './nameColors';
export { isMuted, isDeafened, toggleMute, toggleDeafen } from './audio';
export { activeOverlay, overlayImageUrl, settingsTab, showRoomIds, openSettings, openImage, openConnect, closeOverlay } from './overlay';
export { serverBookmarks, selectedBookmarkId, connectingBookmark, passwordRequested, mediaBaseUrl, loadSettings, addBookmark, updateBookmark, removeBookmark, connectToServer, signingOut, signOutError, signOutDialogOpen, signOut, openSignOut, closeSignOut } from './servers';
export { matrixStatus, matrixConnecting, matrixSessionLive } from './matrixConnection';
export { encryptionStatus, encryptionError, encryptionBusy, encryptionScreen, encryptionScreenDismissable, entryScreen, unlockScreen, hasNoSavedKey, createRecoveryKey, confirmRecoveryKeySaved, submitRecoveryKey, resetEncryption, openEncryptionDialog, openEncryptionReset, dismissEncryptionScreen } from './encryption';
export type { EncryptionScreen } from './encryption';
export { replyingTo, setReply, clearReply, editingMessage, setEditing, clearEditing, setDrafting } from './compose';
export { voiceChannels, voiceUsers, voiceConnected, mumbleStatus, usersByChannel, talkingUsers, certChangeRequest } from './voiceState';
export type { VoiceChannel, VoiceUser, MumbleStatus } from './voiceState';
export { errorLog, toastError, showToast } from './errors';
export type { ErrorEntry } from './errors';
export { userVolumes, setUserVolume } from './userVolumes';
export { sfxVolume, playSfx, setSfxDeafened } from './sfx';
export type { SfxName } from './sfx';
export { transmissionMode, setTransmissionMode, vadThreshold, setVadThreshold, voiceHold, setVoiceHold, useMumbleSettings, setUseMumbleSettings, deafenSuppressesNotifs, setDeafenSuppressesNotifs } from './voiceSettings';
export type { TransmissionMode } from './voiceSettings';
export { theme, initTheme } from './theme';
export { compactChat, initLayout } from './layout';
export { followNewMessages, setFollowNewMessages } from './followMessages';
export { desktopNotifications, setDesktopNotifications } from './desktopNotifications';
export { sidebarCollapsed, sidebarTransitioning, peekSuppressed, toggleSidebar, initCursorTracking, destroySidebar } from './sidebar';
export type { Theme } from './theme';
export { updateStatus, updateVersion, updateError, checkForUpdate, restartApp } from './updater';
export type { UpdateStatus } from './updater';
export { initTray, destroyTray } from './tray';

export function initStores(): void {
    initEventRouter();
    initChannels();
}
