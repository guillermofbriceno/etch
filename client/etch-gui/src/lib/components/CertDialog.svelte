<script lang="ts">
    import { certChangeRequest } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';

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

    function handleKeydown(event: KeyboardEvent) {
        if (!$certChangeRequest) return;
        if (event.key === 'Escape') {
            event.stopPropagation();
            handleReject();
        }
    }
</script>

<svelte:window on:keydown={handleKeydown} />

{#if $certChangeRequest}
    <div class="cert-backdrop">
        <button class="backdrop-close" on:click={handleReject} aria-label="Close dialog"></button>
        <div class="cert-dialog" role="dialog" aria-modal="true">
            <h3>Certificate Changed</h3>
            <p class="cert-prompt">
                The voice server certificate for <strong>{$certChangeRequest.host}:{$certChangeRequest.port}</strong> has changed.
                This could indicate a server reconfiguration or a potential security issue.
            </p>
            <div class="fingerprint">
                <span class="fingerprint-label">New fingerprint:</span>
                <code>{formatFingerprint($certChangeRequest.new_fingerprint)}</code>
            </div>
            <div class="cert-actions">
                <button class="action-btn accept-btn" on:click={handleAccept}>Accept</button>
                <button class="action-btn reject-btn" on:click={handleReject}>Reject</button>
            </div>
        </div>
    </div>
{/if}

<style>
    .cert-backdrop {
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

    .cert-dialog {
        position: relative;
        z-index: 1;
        background-color: var(--bg-tertiary);
        border-radius: 8px;
        padding: 32px;
        width: 480px;
        max-width: 90vw;
    }

    .cert-dialog h3 {
        color: #fff;
        font-size: 18px;
        font-weight: 600;
        margin: 0 0 8px;
    }

    .cert-prompt {
        color: #b9bbbe;
        font-size: 14px;
        margin: 0 0 20px;
        line-height: 1.4;
    }

    .cert-prompt strong {
        color: #dcddde;
    }

    .fingerprint {
        background-color: var(--bg-input);
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 12px;
        margin-bottom: 20px;
    }

    .fingerprint-label {
        display: block;
        color: #b9bbbe;
        font-size: 12px;
        margin-bottom: 6px;
    }

    .fingerprint code {
        color: #dcddde;
        font-size: 12px;
        word-break: break-all;
        font-family: 'JetBrains Mono', monospace;
    }

    .cert-actions {
        display: flex;
        gap: 12px;
    }

    .action-btn {
        padding: 8px 20px;
        border: none;
        border-radius: 4px;
        font-size: 14px;
        font-family: 'Inter', sans-serif;
        font-weight: 500;
        cursor: pointer;
        transition: background-color 0.15s;
    }

    .accept-btn { background-color: #43b581; color: #fff; }
    .accept-btn:hover { background-color: #3ca374; }

    .reject-btn { background-color: #4f545c; color: #fff; }
    .reject-btn:hover { background-color: #5d6269; }
</style>
