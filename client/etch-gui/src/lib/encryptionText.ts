import type { EncryptionStatus } from '$lib/types';

type Status = EncryptionStatus['type'];

// One name per action, shared by every control that leads to it and by the help that points to it.
export const ACTION = {
    createKey: 'Create Recovery Key',
    enterKey: 'Enter Recovery Key',
    replaceKey: 'Replace Recovery Key',
    setUpDevice: 'Set Up This Device',
    reset: 'Reset Encryption',
} as const;

export const TITLE = {
    'create-key': 'Create Your Recovery Key',
    'save-key': 'Save Your Recovery Key',
    'enter-key': 'Enter Your Recovery Key',
    'set-up-device': ACTION.setUpDevice,
    'replace-key': 'Replace Your Recovery Key',
    reset: ACTION.reset,
} as const;

export type EncryptionScreen = keyof typeof TITLE;

type StatusText = {
    summary: string;
    /** Names the control that opens the dialog, where the status has one. */
    button: string | null;
    /** Starts a sentence about what this device gains, where it cannot use encrypted rooms yet. */
    remedy: string | null;
};

const STATUS: Record<Status, StatusText> = {
    Unknown: {
        summary: 'Etch is still checking this account.',
        button: null,
        remedy: null,
    },
    Ready: {
        summary: 'Your recovery key is set up. You need it to read your encrypted messages when you sign in on a new device.',
        button: ACTION.replaceKey,
        remedy: null,
    },
    NeedsRecoverySetup: {
        summary: 'You have not created a recovery key yet.',
        button: ACTION.createKey,
        remedy: null,
    },
    RecoveryKeyPending: {
        summary: 'Your new recovery key is waiting to be saved.',
        button: null,
        remedy: null,
    },
    NeedsRecoveryKey: {
        summary: 'This device needs your recovery key to read your encrypted messages.',
        button: ACTION.enterKey,
        remedy: 'Enter your recovery key',
    },
    NeedsVerifiedDevice: {
        summary: 'This device cannot read your encrypted messages yet.',
        button: ACTION.setUpDevice,
        remedy: 'Set up this device',
    },
};

export const NOT_CONNECTED = 'Connect to a server to manage your recovery key.';

export function encryptionText(status: Status): StatusText {
    return STATUS[status];
}

export type UndecryptableLine = { text: string; remedy: string | null };

export function undecryptableLine(count: number, status: Status): UndecryptableLine {
    const messages = count === 1 ? '1 encrypted message' : `${count} encrypted messages`;
    const remedy = STATUS[status].remedy;
    if (remedy === null) return { text: `${messages} cannot be read on this device.`, remedy: null };
    return { text: `${messages}.`, remedy: `${remedy} to read ${count === 1 ? 'it' : 'them'}.` };
}
