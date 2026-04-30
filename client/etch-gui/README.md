# Tauri + SvelteKit + TypeScript

This template should help get you started developing with Tauri, SvelteKit and TypeScript in Vite.

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).

## Testing

Tests use Vitest with jsdom. No Tauri runtime required. All Tauri APIs are mocked globally.

```bash
# Enter nix dev environment first
nix develop

# Run once
cd client/etch-gui && pnpm test

# Watch mode
cd client/etch-gui && pnpm test:watch
```

The suite covers three layers:

- **Store tests** -- Call event handlers directly and assert on store values. Covers voice state, channels, messages, errors, user profiles, overlay state, and user volumes.
- **IPC command tests** -- Call store actions and verify the outbound `invoke('core_command', ...)` payloads. Covers audio, messages, servers, voice settings, channels, and user volumes.
- **Component tests** -- Render real Svelte components, simulate user interaction, and assert on DOM output and IPC calls.

For detailed information on test patterns, file layout, mocking strategy, and known gotchas, see [`src/lib/stores/__tests__/README.md`](src/lib/stores/__tests__/README.md).
