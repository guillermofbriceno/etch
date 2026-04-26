<script lang="ts">
    import { connectingBookmark, passwordRequested, connectToServer } from '$lib/stores';

    let passwordInput = '';
    let error = '';

    // React to password request events from the core
    $: if ($passwordRequested) {
        error = '';
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
        error = '';
        connectingBookmark.set(null);
    }

    function handleKeydown(event: KeyboardEvent) {
        if (!$passwordRequested) return;
        if (event.key === 'Escape') {
            event.stopPropagation();
            handleCancel();
        }
    }
</script>

<svelte:window on:keydown={handleKeydown} />

{#if $passwordRequested && $connectingBookmark}
    <div class="password-backdrop" on:click={handleCancel}>
        <div class="password-dialog" on:click|stopPropagation>
            <h3>Password Required</h3>
            <p class="password-prompt">
                Enter the password for <strong>{$connectingBookmark.username}</strong> on <strong>{$connectingBookmark.address}</strong>
            </p>
            {#if error}
                <p class="error-message">{error}</p>
            {/if}
            <input
                type="password"
                bind:value={passwordInput}
                placeholder="Password"
                on:keydown={(e) => { if (e.key === 'Enter') handleSubmit(); }}
                autofocus
            />
            <div class="password-actions">
                <button class="action-btn login-btn" on:click={handleSubmit}>Login</button>
                <button class="action-btn cancel-btn" on:click={handleCancel}>Cancel</button>
            </div>
        </div>
    </div>
{/if}

<style>
    .password-backdrop {
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

    .password-dialog {
        background-color: var(--bg-tertiary);
        border-radius: 8px;
        padding: 32px;
        width: 400px;
        max-width: 90vw;
    }

    .password-dialog h3 {
        color: #fff;
        font-size: 18px;
        font-weight: 600;
        margin: 0 0 8px;
    }

    .password-prompt {
        color: #b9bbbe;
        font-size: 14px;
        margin: 0 0 20px;
    }

    .password-prompt strong {
        color: #dcddde;
    }

    .error-message {
        color: #ed4245;
        font-size: 14px;
        margin: 0 0 12px;
    }

    .password-dialog input[type="password"] {
        width: 100%;
        background-color: var(--bg-input);
        color: #dcddde;
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 10px;
        font-size: 16px;
        font-family: 'Inter', sans-serif;
        outline: none;
        box-sizing: border-box;
        margin-bottom: 20px;
    }

    .password-dialog input[type="password"]:focus {
        border-color: #7289da;
    }

    .password-actions {
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

    .login-btn { background-color: #43b581; color: #fff; }
    .login-btn:hover { background-color: #3ca374; }

    .cancel-btn { background-color: #4f545c; color: #fff; }
    .cancel-btn:hover { background-color: #5d6269; }
</style>
