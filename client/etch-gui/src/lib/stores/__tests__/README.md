# Frontend Test Suite

Tests run on **Vitest + jsdom**, sharing the Vite config. No Tauri runtime needed.

```bash
# Run once
cd client/etch-gui && pnpm test

# Watch mode
cd client/etch-gui && pnpm test:watch
```

Requires `nix develop` for pnpm.

## How it works

All Tauri API modules are mocked globally in `src/lib/test-setup.ts` using `vi.mock()`. These mocks are hoisted before any imports, so stores and components that call `invoke`, `listen`, etc. at the module level just get no-ops. The `invoke` mock is the main one you'll spy on to verify outbound IPC commands.

Store singletons persist across tests. Call `resetStores()` from `helpers.ts` in your `beforeEach` to put everything back to initial values. This includes a trick for voiceState: firing a `Disconnected` event to reset the private `settled` and `localSession` variables that aren't exported.

Component tests use `@testing-library/svelte` for rendering and `@testing-library/user-event` for realistic keyboard/mouse simulation. The `svelteTesting()` Vite plugin handles automatic component cleanup between tests. DOM matchers like `toBeInTheDocument()` come from `@testing-library/jest-dom`.

## File layout

```
src/lib/
├── test-setup.ts              # Global Tauri API mocks (runs before every test file)
├── stores/__tests__/
│   ├── helpers.ts             # resetStores() utility
│   ├── voiceState.test.ts     # Mumble event handling, voice store state
│   ├── messages.test.ts       # Timeline manipulation, activeWindow, notification sounds
│   ├── channels.test.ts       # Channel list, DM hiding/unhiding, auto-select, unread counts
│   ├── errors.test.ts         # Error log accumulation, toast timer lifecycle
│   ├── user.test.ts           # CurrentUser handling, profile update fallback behavior
│   ├── overlay.test.ts        # Overlay state machine transitions
│   ├── ipc-events.test.ts     # Event router: CoreEvent dispatch to stores
│   ├── ipc-commands-messages.test.ts
│   ├── ipc-commands-audio.test.ts
│   ├── ipc-commands-voiceSettings.test.ts
│   ├── ipc-commands-servers.test.ts  # Also covers SettingsLoaded hydration, Matrix connection state
│   ├── ipc-commands-channels.test.ts
│   └── ipc-commands-userVolumes.test.ts
└── components/__tests__/
    ├── MessageInput.test.ts   # Send, compose lock, reply, emoji, tab completion
    └── VoiceUserList.test.ts  # Icon states, user names, context menu
```

**Store tests** call handler functions directly (`handleMatrixEvent`, `handleMumbleEvent`) and assert on store values with `get()`.

**IPC command tests** call store actions (`sendMessage`, `toggleMute`, etc.) and verify the correct `invoke('core_command', ...)` payload was sent.

**IPC event tests** capture the `listen()` callback from `initEventRouter()`, fire `CoreEvent` payloads through it, and verify the right stores update.

**Component tests** render real components with real stores (not mocked), interact via `userEvent`, and assert on DOM content and `invoke` calls.

## Things to know

**Don't import from the barrel `$lib/stores/index.ts` in store tests.** It re-exports from `updater.ts`, which calls `listen()` at the module level. Import directly from the specific store file instead (e.g., `../voiceState`). Component tests can safely import from `$lib/stores` since `listen` is already mocked.

**The `windows` store in messages.ts is private.** You can't reset it with `resetStores()`. If your test populates a room's timeline, clear it in `beforeEach` with:
```ts
handleMatrixEvent({ type: 'TimelineCleared', data: 'room1' } as any);
```

**`initChannels()` creates store subscriptions that stack.** Call it once in `beforeAll`, not `beforeEach`. See `channels.test.ts` for the pattern.

**jsdom limitations.** `scrollHeight` and `scrollTop` are always 0, `ResizeObserver` doesn't exist, and clipboard access is limited. Don't try to test scroll behavior or textarea auto-resize. Those are better suited for browser-mode tests later.

**String literal types.** When building test payloads, TypeScript may widen `'Text'` to `string`. Use `as const` if you need the literal union type:
```ts
{ etch_room_type: 'Text' as const }
```
