<script lang="ts">
    import { connectingBookmark, passwordRequested, connectToServer } from '$lib/stores';
    import Dialog from './Dialog.svelte';
    import DialogStatus from './DialogStatus.svelte';

    let passwordInput = '';
    let error: string | null = null;

    // React to password request events from the core
    $: if ($passwordRequested) {
        error = null;
        passwordInput = '';
    }

    async function handleSubmit() {
        const bookmark = $connectingBookmark;
        if (!bookmark) return;
        passwordRequested.set(false);
        try {
            await connectToServer(bookmark, passwordInput);
        } catch (e) {
            error = `Authentication failed: ${e}`;
            passwordRequested.set(true);
        }
        passwordInput = '';
    }

    function handleCancel() {
        passwordRequested.set(false);
        passwordInput = '';
        error = null;
        connectingBookmark.set(null);
    }
</script>

{#if $passwordRequested && $connectingBookmark}
    <Dialog title="Password Required" on:dismiss={handleCancel}>
        <p class="prompt">
            Enter the password for <strong>{$connectingBookmark.username}</strong> on <strong>{$connectingBookmark.address}</strong>
        </p>
        <input
            type="password"
            class="text-field"
            bind:value={passwordInput}
            placeholder="Password"
            aria-label="Password"
            on:keydown={(e) => { if (e.key === 'Enter') handleSubmit(); }}
        />
        <DialogStatus {error} />
        <div class="actions">
            <button class="action-btn" on:click={handleSubmit}>Login</button>
            <button class="action-btn secondary" on:click={handleCancel}>Cancel</button>
        </div>
    </Dialog>
{/if}
