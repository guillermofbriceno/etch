import { vi } from 'vitest';
import { voiceChannels, voiceUsers, talkingUsers, mumbleStatus } from '../voiceState';
import { handleMumbleEvent } from '../voiceState';
import { channels } from '../channels';
import { activeChannelId } from '../activeChannel';
import { isMuted, isDeafened } from '../audio';
import { currentUser } from '../user';
import { errorLog, toastError } from '../errors';
import { userVolumes } from '../userVolumes';
import { transmissionMode, vadThreshold, voiceHold, useMumbleSettings } from '../voiceSettings';
import { activeOverlay, overlayImageUrl, settingsTab, showRoomIds } from '../overlay';
import { replyingTo } from '../compose';
import { serverBookmarks, selectedBookmarkId, connectingBookmark, passwordRequested, matrixConnecting, mediaBaseUrl } from '../servers';

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

    // Channels
    channels.set([]);
    activeChannelId.set(null);

    // Audio
    isMuted.set(false);
    isDeafened.set(false);

    // User
    currentUser.set({ username: '', matrixId: '', displayName: null, avatarUrl: null });

    // Errors
    errorLog.set([]);
    toastError.set(null);

    // Voice settings
    userVolumes.set({});
    transmissionMode.set('voice_activation');
    vadThreshold.set(60);
    voiceHold.set(250);
    useMumbleSettings.set(false);

    // Overlay
    activeOverlay.set('none');
    overlayImageUrl.set(null);
    settingsTab.set('voice');
    showRoomIds.set(false);

    // Compose
    replyingTo.set(null);

    // Servers
    serverBookmarks.set([]);
    selectedBookmarkId.set(null);
    connectingBookmark.set(null);
    passwordRequested.set(false);
    matrixConnecting.set(false);
    mediaBaseUrl.set(null);
}
