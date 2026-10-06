import { vi } from 'vitest';
import { voiceChannels, voiceUsers, talkingUsers, mumbleStatus, certChangeRequest } from '../voiceState';
import { handleMumbleEvent } from '../voiceState';
import { channels, dmLastActivity } from '../channels';
import { activeChannelId } from '../activeChannel';
import { isMuted, isDeafened } from '../audio';
import { currentUser } from '../user';
import { resetNameColors, canSetNameColor } from '../nameColors';
import { errorLog, toastError } from '../errors';
import { userVolumes } from '../userVolumes';
import { transmissionMode, vadThreshold, voiceHold, useMumbleSettings, deafenSuppressesNotifs } from '../voiceSettings';
import { activeOverlay, overlayImageUrl, settingsTab, showRoomIds } from '../overlay';
import { replyingTo, editingMessage } from '../compose';
import { serverBookmarks, selectedBookmarkId, connectingBookmark, passwordRequested, matrixConnecting, matrixConnected, mediaBaseUrl, signingOut, signOutError, signOutDialogOpen } from '../servers';
import { encryptionStatus, encryptionError, encryptionPromptDismissed, encryptionRequestedScreen, encryptionBusy } from '../encryption';
import { compactChat } from '../layout';

/**
 * Reset all stores to their initial values.
 * Call in beforeEach() for any test that modifies store state.
 */
export function resetStores(): void {
    // Voice -- Disconnected event resets the private `settled` and `localSession` vars
    handleMumbleEvent({ type: 'ConnectionState', data: { type: 'Disconnected' } } as any);
    voiceChannels.set(new Map());
    voiceUsers.set(new Map());
    talkingUsers.set(new Set());
    mumbleStatus.set('disconnected');
    certChangeRequest.set(null);

    // Channels
    channels.set([]);
    activeChannelId.set(null);
    dmLastActivity.set({});

    // Audio
    isMuted.set(false);
    isDeafened.set(false);

    // User
    currentUser.set({ username: '', matrixId: '', displayName: null, avatarUrl: null });

    // Name colors -- also clears the private record of requests awaiting an answer
    resetNameColors();
    canSetNameColor.set(false);

    // Errors
    errorLog.set([]);
    toastError.set(null);

    // Voice settings
    userVolumes.set({});
    transmissionMode.set('voice_activation');
    vadThreshold.set(60);
    voiceHold.set(250);
    useMumbleSettings.set(false);
    deafenSuppressesNotifs.set(true);

    // Overlay
    activeOverlay.set('none');
    overlayImageUrl.set(null);
    settingsTab.set('voice');
    showRoomIds.set(false);

    // Compose
    replyingTo.set(null);
    editingMessage.set(null);

    // Servers
    serverBookmarks.set([]);
    selectedBookmarkId.set(null);
    connectingBookmark.set(null);
    passwordRequested.set(false);
    matrixConnecting.set(false);
    matrixConnected.set(false);
    mediaBaseUrl.set(null);
    signingOut.set(false);
    signOutError.set(null);
    signOutDialogOpen.set(false);

    // Encryption
    encryptionStatus.set({ type: 'Unknown' });
    encryptionError.set(null);
    encryptionPromptDismissed.set(null);
    encryptionRequestedScreen.set(null);
    encryptionBusy.set(false);

    // Layout
    compactChat.set(false);
}
