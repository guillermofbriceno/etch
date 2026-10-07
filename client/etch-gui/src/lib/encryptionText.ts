import type { EncryptionStatus } from '$lib/types';
import type { EncryptionScreen, EntryScreen, UnlockScreen } from '$lib/stores/encryption';

type Status = EncryptionStatus['type'];

// One name per action, shared by the control that opens its screen, the button that carries it out, and the help that points to it.
export const ACTION: Record<EntryScreen, string> = {
    'create-key': 'Create Recovery Key',
    'enter-key': 'Enter Recovery Key',
    'replace-key': 'Replace Recovery Key',
    'set-up-device': 'Set Up This Device',
    reset: 'Reset Encryption',
};

export const TITLE: Record<EncryptionScreen, string> = {
    'create-key': 'Create Your Recovery Key',
    'save-key': 'Save Your Recovery Key',
    'enter-key': 'Enter Your Recovery Key',
    'set-up-device': ACTION['set-up-device'],
    'replace-key': 'Replace Your Recovery Key',
    reset: ACTION.reset,
};

export const SUMMARY: Record<Status, string> = {
    Unknown: 'Etch is still checking this account.',
    Ready: 'Your recovery key is set up. You need it to read your encrypted messages when you sign in on a new device.',
    NeedsRecoverySetup: 'You have not saved a recovery key yet.',
    RecoveryKeyPending: 'Your new recovery key is waiting to be saved.',
    NeedsRecoveryKey: 'This device needs your recovery key to read your encrypted messages.',
    NeedsVerifiedDevice: 'This device cannot read your encrypted messages yet.',
};

export const NOT_CONNECTED = 'Connect to a server to manage your recovery key.';

// Each starts a sentence about what the device gains.
const REMEDY: Record<UnlockScreen, string> = {
    'enter-key': 'Enter your recovery key',
    'set-up-device': 'Set up this device',
};

export type UndecryptableLine = { text: string; remedy: string | null };

export function undecryptableLine(count: number, unlock: UnlockScreen | null): UndecryptableLine {
    const messages = count === 1 ? '1 encrypted message' : `${count} encrypted messages`;
    if (unlock === null) return { text: `${messages} cannot be read on this device.`, remedy: null };
    return { text: `${messages}.`, remedy: `${REMEDY[unlock]} to read ${count === 1 ? 'it' : 'them'}.` };
}
