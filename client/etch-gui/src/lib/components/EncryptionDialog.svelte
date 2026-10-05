<script lang="ts">
    import {
        encryptionStatus, encryptionError, encryptionPromptDismissed, encryptionResetOpen, encryptionBusy,
        matrixConnected, currentUser,
        createRecoveryKey, confirmRecoveryKeySaved, submitRecoveryKey, resetEncryption,
        dismissEncryptionPrompt, openEncryptionReset, closeEncryptionReset,
    } from '$lib/stores';
    import { saveFileAs } from '$lib/media';
    import type { EncryptionStatus } from '$lib/types';

    type Screen = 'none' | 'create-key' | 'save-key' | 'enter-key' | 'no-key-to-enter' | 'reset';

    let keyInput = '';
    let passwordInput = '';
    let showNoKeyHelp = false;
    let copyLabel = 'Copy';
    let saveNote = '';
    let lastPrompt: EncryptionStatus['type'] | null = null;

    $: status = $encryptionStatus;
    $: pendingKey = status.type === 'RecoveryKeyPending' ? status.data.key : null;
    $: screen = screenFor(status.type, $matrixConnected, $encryptionPromptDismissed, $encryptionResetOpen);
    $: canDismiss = screen === 'reset' || screen === 'enter-key' || screen === 'no-key-to-enter'
        || (screen === 'create-key' && $encryptionError !== null);
    $: forgetInputsOnNewPrompt(status.type);
    $: if (!$encryptionResetOpen) passwordInput = '';

    function screenFor(type: EncryptionStatus['type'], connected: boolean, dismissed: boolean, resetOpen: boolean): Screen {
        if (!connected) return 'none';
        if (type === 'RecoveryKeyPending') return 'save-key';
        if (resetOpen) return 'reset';
        if (dismissed) return 'none';
        if (type === 'NeedsRecoverySetup') return 'create-key';
        if (type === 'NeedsRecoveryKey') return 'enter-key';
        if (type === 'NeedsVerifiedDevice') return 'no-key-to-enter';
        return 'none';
    }

    // A reconnect passes through Unknown and back, which must not clear a half-typed key.
    function forgetInputsOnNewPrompt(type: EncryptionStatus['type']) {
        if (type === 'Unknown' || type === lastPrompt) return;
        lastPrompt = type;
        keyInput = '';
        showNoKeyHelp = false;
        copyLabel = 'Copy';
        saveNote = '';
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

    function dismiss() {
        if (screen === 'reset') closeEncryptionReset();
        else if (canDismiss) dismissEncryptionPrompt();
    }

    async function copyKey() {
        if (!pendingKey) return;
        try {
            await navigator.clipboard.writeText(pendingKey);
            copyLabel = 'Copied';
        } catch {
            copyLabel = 'Could not copy';
        }
        setTimeout(() => copyLabel = 'Copy', 2000);
    }

    function keyFileText(key: string): string {
        const account = $currentUser.matrixId ? ` for ${$currentUser.matrixId}` : '';
        return `Etch recovery key${account}\n\n${key}\n\n`
            + 'Keep this file somewhere safe. You need this key to read your encrypted messages when you sign in on a new device.\n';
    }

    async function saveKey() {
        if (!pendingKey) return;
        saveNote = '';
        try {
            const saved = await saveFileAs('etch-recovery-key.txt', new TextEncoder().encode(keyFileText(pendingKey)));
            if (saved) saveNote = 'Saved.';
        } catch (e) {
            saveNote = `Could not save the file: ${e}`;
        }
    }

    // In the capture phase, so Escape does not also close the Settings overlay this can sit above.
    function handleKeydown(event: KeyboardEvent) {
        if (screen === 'none' || event.key !== 'Escape') return;
        event.stopPropagation();
        dismiss();
    }
</script>

<svelte:window on:keydown|capture={handleKeydown} />

{#if screen !== 'none'}
    <div class="encryption-backdrop">
        {#if canDismiss}
            <button class="backdrop-close" on:click={dismiss} aria-label="Close dialog"></button>
        {/if}
        <div class="encryption-dialog" role="dialog" aria-modal="true" aria-labelledby="encryption-dialog-title">
            {#if screen === 'create-key'}
                <h3 id="encryption-dialog-title">Save your recovery key</h3>
                <p class="prompt">
                    Your messages here are encrypted. A recovery key lets you read them when you sign in on another device.
                </p>
                <p class="prompt">Create yours now and keep it somewhere safe.</p>
                {#if $encryptionError}
                    <p class="error-message" role="alert">{$encryptionError}</p>
                {/if}
                <div class="actions">
                    <button class="action-btn primary-btn" on:click={createRecoveryKey} disabled={$encryptionBusy}>
                        {$encryptionBusy ? 'Creating...' : $encryptionError ? 'Try again' : 'Create recovery key'}
                    </button>
                    {#if $encryptionError}
                        <button class="action-btn secondary-btn" on:click={dismissEncryptionPrompt}>Not now</button>
                    {/if}
                </div>
            {:else if screen === 'save-key'}
                <h3 id="encryption-dialog-title">Your recovery key</h3>
                <p class="prompt">
                    Keep this somewhere safe, such as a password manager. You need it to read your encrypted messages when you sign in on a new device.
                </p>
                <p class="prompt">Etch cannot show this key again.</p>
                <code class="recovery-key">{pendingKey}</code>
                <div class="actions">
                    <button class="action-btn secondary-btn" on:click={copyKey}>{copyLabel}</button>
                    <button class="action-btn secondary-btn" on:click={saveKey}>Save to file</button>
                </div>
                {#if saveNote}
                    <p class="note">{saveNote}</p>
                {/if}
                {#if $encryptionError}
                    <p class="error-message" role="alert">{$encryptionError}</p>
                {/if}
                <div class="actions confirm">
                    <button class="action-btn primary-btn" on:click={confirmRecoveryKeySaved} disabled={$encryptionBusy}>I have saved it</button>
                </div>
            {:else if screen === 'enter-key'}
                <h3 id="encryption-dialog-title">Enter your recovery key</h3>
                <p class="prompt">
                    Enter your recovery key to read your encrypted messages on this device. Voice works without it.
                </p>
                {#if $encryptionError}
                    <p class="error-message" role="alert">{$encryptionError}</p>
                {/if}
                <!-- svelte-ignore a11y_autofocus -->
                <input
                    type="text"
                    class="text-field"
                    bind:value={keyInput}
                    placeholder="Recovery key"
                    aria-label="Recovery key"
                    autocomplete="off"
                    spellcheck="false"
                    on:keydown={(e) => { if (e.key === 'Enter') submitKey(); }}
                    autofocus
                />
                <div class="actions">
                    <button class="action-btn primary-btn" on:click={submitKey} disabled={$encryptionBusy || !keyInput.trim()}>
                        {$encryptionBusy ? 'Checking...' : 'Continue'}
                    </button>
                    <button class="action-btn secondary-btn" on:click={dismissEncryptionPrompt}>Not now</button>
                </div>
                <button class="link-btn" on:click={() => showNoKeyHelp = !showNoKeyHelp} aria-expanded={showNoKeyHelp}>
                    I don't have my key
                </button>
                {#if showNoKeyHelp}
                    <div class="help">
                        <p>
                            If you are still signed in to Etch on another device, open Settings there, go to My Account, and generate a new recovery key. Then enter it here.
                        </p>
                        <p>
                            If you have no other device, you can reset encryption. Older encrypted messages that this device cannot already read will be lost for good.
                        </p>
                        <button class="action-btn danger-btn" on:click={openEncryptionReset}>Reset encryption</button>
                    </div>
                {/if}
            {:else if screen === 'no-key-to-enter'}
                <h3 id="encryption-dialog-title">Encryption is not set up on this device</h3>
                <p class="prompt">
                    This device cannot read your encrypted messages yet, and your account has no recovery key to enter. Voice works without it.
                </p>
                <p class="prompt">
                    If you are signed in to Etch on another device, create a recovery key there. Etch will then ask for it here.
                </p>
                <p class="prompt">
                    If you have no other device, you can reset encryption. Older encrypted messages that this device cannot already read will be lost for good.
                </p>
                <div class="actions">
                    <button class="action-btn danger-btn" on:click={openEncryptionReset}>Reset encryption</button>
                    <button class="action-btn secondary-btn" on:click={dismissEncryptionPrompt}>Not now</button>
                </div>
            {:else if screen === 'reset'}
                <h3 id="encryption-dialog-title">Reset encryption</h3>
                <p class="prompt">This gives your account a fresh start for encryption. It cannot be undone.</p>
                <ul class="consequences">
                    <li>Older encrypted messages that this device cannot already read are lost for good.</li>
                    <li>Your current recovery key stops working.</li>
                    <li>Your other devices will ask for the new recovery key.</li>
                </ul>
                <p class="warning">
                    The reset starts as soon as you press the button. If the password is wrong, your current recovery key still stops working.
                </p>
                {#if $encryptionError}
                    <p class="error-message" role="alert">{$encryptionError}</p>
                {/if}
                <!-- svelte-ignore a11y_autofocus -->
                <input
                    type="password"
                    class="text-field"
                    bind:value={passwordInput}
                    placeholder="Account password"
                    aria-label="Account password"
                    autofocus
                />
                <div class="actions">
                    <button class="action-btn danger-btn" on:click={submitReset} disabled={$encryptionBusy || !passwordInput}>
                        {$encryptionBusy ? 'Resetting...' : 'Reset encryption'}
                    </button>
                    <button class="action-btn secondary-btn" on:click={closeEncryptionReset}>Cancel</button>
                </div>
            {/if}
        </div>
    </div>
{/if}

<style>
    .encryption-backdrop {
        position: fixed;
        top: 0;
        left: 0;
        width: 100vw;
        height: 100vh;
        background-color: rgba(0, 0, 0, 0.7);
        z-index: 10000;
        display: flex;
        align-items: center;
        justify-content: center;
    }

    .backdrop-close {
        position: absolute;
        inset: 0;
        background: none;
        border: none;
        cursor: default;
    }

    .encryption-dialog {
        position: relative;
        z-index: 1;
        background-color: var(--bg-tertiary);
        border-radius: 8px;
        padding: 32px;
        width: 480px;
        max-width: 90vw;
        max-height: 90vh;
        overflow-y: auto;
        box-sizing: border-box;
    }

    .encryption-dialog h3 {
        color: var(--text-bright);
        font-size: 18px;
        font-weight: 600;
        margin: 0 0 8px;
    }

    .prompt,
    .help p {
        color: var(--text-secondary);
        font-size: var(--font-size-base);
        line-height: 1.4;
        margin: 0 0 12px;
    }

    .error-message {
        color: var(--status-danger);
        font-size: var(--font-size-base);
        margin: 0 0 12px;
    }

    .note {
        color: var(--text-tertiary);
        font-size: 13px;
        margin: 8px 0 0;
    }

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
        margin: 8px 0 20px;
    }

    .text-field:focus {
        border-color: var(--primary);
    }

    .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 12px;
        margin-top: 8px;
    }

    .actions.confirm {
        margin-top: 24px;
    }

    .action-btn {
        padding: 8px 20px;
        border: none;
        border-radius: 4px;
        color: var(--text-bright);
        font-size: var(--font-size-base);
        font-family: 'Inter', sans-serif;
        font-weight: 500;
        cursor: pointer;
        transition: background-color 0.15s;
    }

    .action-btn:disabled {
        opacity: 0.4;
        cursor: default;
    }

    .primary-btn { background-color: var(--primary); }
    .primary-btn:hover:not(:disabled) { background-color: var(--primary-hover); }

    .secondary-btn { background-color: var(--bg-active); }
    .secondary-btn:hover:not(:disabled) { background-color: rgba(255, 255, 255, 0.12); }

    .danger-btn { background-color: var(--status-danger); }
    .danger-btn:hover:not(:disabled) { background-color: #c93b3e; }

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

    .warning {
        color: var(--status-warning);
        font-size: var(--font-size-base);
        line-height: 1.4;
        margin: 0 0 12px;
    }
</style>
