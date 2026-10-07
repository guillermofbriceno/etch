<script lang="ts">
    import {
        encryptionStatus, encryptionError, encryptionBusy, encryptionScreen, encryptionScreenDismissable, currentUser,
        createRecoveryKey, confirmRecoveryKeySaved, submitRecoveryKey, resetEncryption,
        openEncryptionReset, dismissEncryptionScreen,
    } from '$lib/stores';
    import { saveFileAs } from '$lib/media';
    import { ACTION, TITLE, SUMMARY } from '$lib/encryptionText';
    import type { EncryptionStatus } from '$lib/types';
    import Dialog from './Dialog.svelte';
    import DialogStatus from './DialogStatus.svelte';

    let keyInput = '';
    let passwordInput = '';
    let showNoKeyHelp = false;
    let keyNote = '';
    let keyError: string | null = null;
    let lastPrompt: EncryptionStatus['type'] | null = null;

    $: status = $encryptionStatus;
    $: summary = SUMMARY[status.type];
    $: pendingKey = status.type === 'RecoveryKeyPending' ? status.data.key : null;
    $: screen = $encryptionScreen;
    $: forgetInputsOnNewPrompt(status.type);
    $: if (screen !== 'reset') passwordInput = '';

    // A reconnect passes through Unknown and back, which must not clear a half-typed key.
    function forgetInputsOnNewPrompt(type: EncryptionStatus['type']) {
        if (type === 'Unknown' || type === lastPrompt) return;
        lastPrompt = type;
        keyInput = '';
        showNoKeyHelp = false;
        keyNote = '';
        keyError = null;
    }

    function submitKey() {
        const key = keyInput.trim();
        if (!key || $encryptionBusy) return;
        submitRecoveryKey(key);
    }

    function submitReset() {
        if (!passwordInput || $encryptionBusy) return;
        resetEncryption(passwordInput);
        passwordInput = '';
    }

    async function copyKey() {
        if (!pendingKey) return;
        keyNote = '';
        keyError = null;
        try {
            await navigator.clipboard.writeText(pendingKey);
            keyNote = 'Copied!';
        } catch {
            keyError = 'Could not copy the key.';
        }
    }

    function keyFileText(key: string): string {
        const account = $currentUser.matrixId ? ` for ${$currentUser.matrixId}` : '';
        return `Etch recovery key${account}\n\n${key}\n\n`
            + 'Keep this file somewhere safe. You need this key to read your encrypted messages when you sign in on a new device.\n';
    }

    async function saveKey() {
        if (!pendingKey) return;
        keyNote = '';
        keyError = null;
        try {
            const saved = await saveFileAs('etch-recovery-key.txt', new TextEncoder().encode(keyFileText(pendingKey)));
            if (saved) keyNote = 'Saved!';
        } catch (e) {
            keyError = `Could not save the file: ${e}`;
        }
    }
</script>

{#if screen}
    <Dialog title={TITLE[screen]} dismissable={$encryptionScreenDismissable} {screen} on:dismiss={dismissEncryptionScreen}>
        {#if screen === 'create-key'}
            <p class="prompt">
                Your messages here are encrypted. A recovery key lets you read them when you sign in on a new device.
            </p>
            <p class="prompt">Create yours now and keep it somewhere safe.</p>
            <DialogStatus error={$encryptionError} note={$encryptionBusy ? 'Creating...' : ''} />
            <div class="actions">
                <button class="action-btn" on:click={createRecoveryKey} disabled={$encryptionBusy}>
                    {$encryptionError ? 'Try Again' : ACTION['create-key']}
                </button>
                {#if $encryptionError}
                    <button class="action-btn secondary" on:click={dismissEncryptionScreen}>Not Now</button>
                {/if}
            </div>
        {:else if screen === 'save-key'}
            <p class="prompt">
                Keep this somewhere safe, such as a password manager. You need it to read your encrypted messages when you sign in on a new device.
            </p>
            <p class="prompt">Etch cannot show this key again.</p>
            <code class="recovery-key">{pendingKey}</code>
            <div class="actions">
                <button class="action-btn secondary" on:click={copyKey}>Copy</button>
                <button class="action-btn secondary" on:click={saveKey}>Save to File</button>
            </div>
            <DialogStatus error={$encryptionError ?? keyError} note={keyNote} />
            <div class="actions">
                <button class="action-btn" on:click={confirmRecoveryKeySaved} disabled={$encryptionBusy}>I have saved it</button>
            </div>
        {:else if screen === 'enter-key'}
            <p class="prompt">{summary} Voice works without it.</p>
            <input
                type="text"
                class="text-field"
                bind:value={keyInput}
                placeholder="Recovery key"
                aria-label="Recovery key"
                autocomplete="off"
                spellcheck="false"
                on:keydown={(e) => { if (e.key === 'Enter') submitKey(); }}
            />
            <DialogStatus error={$encryptionError} note={$encryptionBusy ? 'Checking...' : ''} />
            <div class="actions">
                <button class="action-btn" on:click={submitKey} disabled={$encryptionBusy || !keyInput.trim()}>Continue</button>
                <button class="action-btn secondary" on:click={dismissEncryptionScreen}>Not Now</button>
            </div>
            <button class="link-btn" on:click={() => showNoKeyHelp = !showNoKeyHelp} aria-expanded={showNoKeyHelp}>
                I don't have my key
            </button>
            {#if showNoKeyHelp}
                <div class="help">
                    <p class="prompt">
                        If you are still signed in to Etch on another device, open Settings there, go to My Account, and choose {ACTION['replace-key']}. Then enter the new key here.
                    </p>
                    <p class="prompt">
                        If you have no other device, you can reset encryption. Older encrypted messages that this device cannot already read will be lost for good.
                    </p>
                    <button class="action-btn danger" on:click={openEncryptionReset}>{ACTION.reset}</button>
                </div>
            {/if}
        {:else if screen === 'set-up-device'}
            <p class="prompt">{summary} Your account has no recovery key to enter. Voice works without it.</p>
            <p class="prompt">
                If you are signed in to Etch on another device, create a recovery key there. Etch will then ask for it here.
            </p>
            <p class="prompt">
                If you have no other device, you can reset encryption. Older encrypted messages that this device cannot already read will be lost for good.
            </p>
            <div class="actions">
                <button class="action-btn danger" on:click={openEncryptionReset}>{ACTION.reset}</button>
                <button class="action-btn secondary" on:click={dismissEncryptionScreen}>Not Now</button>
            </div>
        {:else if screen === 'replace-key'}
            <p class="prompt">
                Your current recovery key stops working as soon as you continue. Etch will then show you the new one to save.
            </p>
            <DialogStatus error={$encryptionError} note={$encryptionBusy ? 'Replacing...' : ''} />
            <div class="actions">
                <button class="action-btn" on:click={createRecoveryKey} disabled={$encryptionBusy}>
                    {$encryptionError ? 'Try Again' : ACTION['replace-key']}
                </button>
                <button class="action-btn secondary" on:click={dismissEncryptionScreen} disabled={$encryptionBusy}>Cancel</button>
            </div>
        {:else if screen === 'reset'}
            <p class="prompt">This gives your account a fresh start for encryption. It cannot be undone.</p>
            <ul class="consequences">
                <li>Older encrypted messages that this device cannot already read are lost for good.</li>
                <li>Your current recovery key stops working.</li>
                <li>Your other devices will ask for the new recovery key.</li>
            </ul>
            <p class="prompt warning">
                The reset begins when you press the button. Your current recovery key stops working even if the password turns out to be wrong.
            </p>
            <input
                type="password"
                class="text-field"
                bind:value={passwordInput}
                placeholder="Account password"
                aria-label="Account password"
            />
            <DialogStatus error={$encryptionError} note={$encryptionBusy ? 'Resetting...' : ''} />
            <div class="actions">
                <button class="action-btn danger" on:click={submitReset} disabled={$encryptionBusy || !passwordInput}>{ACTION.reset}</button>
                <button class="action-btn secondary" on:click={dismissEncryptionScreen} disabled={$encryptionBusy}>Cancel</button>
            </div>
        {/if}
    </Dialog>
{/if}

<style>
    .recovery-key {
        display: block;
        background-color: var(--bg-input);
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 12px;
        margin: 8px 0 12px;
        color: var(--text-bright);
        font-family: 'JetBrains Mono', monospace;
        font-size: 14px;
        line-height: 1.6;
        word-break: break-word;
        -webkit-user-select: all;
        user-select: all;
    }

    .text-field {
        width: 100%;
        background-color: var(--bg-input);
        color: var(--text-primary);
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 10px;
        font-size: 16px;
        font-family: 'Inter', sans-serif;
        outline: none;
        box-sizing: border-box;
        margin: 8px 0 4px;
    }

    .text-field:focus {
        border-color: var(--primary);
    }

    .link-btn {
        background: none;
        border: none;
        padding: 0;
        margin-top: 20px;
        color: var(--text-link);
        font-size: var(--font-size-base);
        font-family: 'Inter', sans-serif;
        cursor: pointer;
    }

    .link-btn:hover {
        text-decoration: underline;
    }

    .help {
        margin-top: 12px;
        padding-top: 12px;
        border-top: 1px solid var(--border-medium);
    }

    .consequences {
        color: var(--text-secondary);
        font-size: var(--font-size-base);
        line-height: 1.4;
        margin: 0 0 12px;
        padding-left: 20px;
    }

    .consequences li {
        margin-bottom: 4px;
    }
</style>
