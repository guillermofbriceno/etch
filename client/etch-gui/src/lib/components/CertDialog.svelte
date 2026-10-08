<script lang="ts">
    import { certChangeRequest } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';
    import Dialog from './Dialog.svelte';

    function formatFingerprint(hex: string): string {
        return hex.replace(/(.{2})/g, '$1:').slice(0, -1).toUpperCase();
    }

    async function handleAccept() {
        const req = $certChangeRequest;
        if (!req) return;
        certChangeRequest.set(null);
        await sendCoreCommand({
            type: 'System',
            data: {
                type: 'AcceptMumbleCert',
                data: { host: req.host, port: req.port, fingerprint: req.new_fingerprint }
            }
        });
    }

    function handleReject() {
        certChangeRequest.set(null);
    }
</script>

{#if $certChangeRequest}
    <Dialog title="Certificate Changed" on:dismiss={handleReject}>
        <p class="prompt">
            The voice server certificate for <strong>{$certChangeRequest.host}:{$certChangeRequest.port}</strong> has changed.
            This could indicate a server reconfiguration or a potential security issue.
        </p>
        <div class="fingerprint">
            <span class="fingerprint-label">New fingerprint:</span>
            <code>{formatFingerprint($certChangeRequest.new_fingerprint)}</code>
        </div>
        <div class="actions">
            <button class="action-btn" on:click={handleAccept}>Accept</button>
            <!-- Enter must not accept a certificate the user has not looked at. -->
            <button class="action-btn secondary" on:click={handleReject} data-initial-focus>Reject</button>
        </div>
    </Dialog>
{/if}

<style>
    .fingerprint {
        background-color: var(--bg-input);
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 12px;
        margin: 8px 0 20px;
    }

    .fingerprint-label {
        display: block;
        color: var(--text-secondary);
        font-size: 12px;
        margin-bottom: 6px;
    }

    .fingerprint code {
        color: var(--text-primary);
        font-size: 12px;
        word-break: break-all;
        font-family: 'JetBrains Mono', monospace;
    }
</style>
