# Etch

Etch is an experimental Matrix client that wraps Mumble to create a voice-first desktop communications app. It bridges two proven applications: one with excellent rich-text group chat, and another with high-fidelity, low-latency voice. The goal is to bring back the traditional voice-channel experience of TeamSpeak and Mumble while adding the rich-text messaging that modern users expect.

## Status

Etch is experimental software and is not recommended for real production use.

## Architecture

```
Murmur (voice server) ↔ Mumble (headless, kind of)
                              ↓  etch-bridge plugin
                          etch-core (Rust)
                              ↓  Tauri IPC
                          SvelteKit frontend
```

Matrix handles text chat, rooms, and identity. Mumble handles voice. `etch-core` ties them together behind a single unified interface.

## Project Structure

| Crate / Package | Description |
|---|---|
| `client/etch-core` | Core library — Matrix SDK integration, Mumble process management, audio |
| `client/etch-gui` | Tauri 2 + SvelteKit 2 desktop frontend |
| `client/etch-cli` | CLI client (not implemented) |
| `plugin/etch-bridge` | Mumble plugin (cdylib) that bridges voice data to etch-core over a local socket |
| `shared/bridge-types` | Shared types between the plugin and core |

## Development

Requires [Nix](https://nixos.org/) for the dev environment.

```bash
# Enter dev shell
nix develop

# Install frontend deps
cd client/etch-gui && pnpm install

# Run in dev mode (Tauri + Vite HMR)
cd client/etch-gui && cargo tauri dev

# Build
cd client/etch-gui && cargo tauri build
```

## Custom Mumble Client

Etch requires a patched build of the Mumble client. The stock plugin API doesn't expose enough for our use case. There's no way for a plugin to receive events for user mute/deafen state changes, channel updates, or other session-level details that Etch needs to keep its UI in sync. Our fork adds additional callbacks to the plugin API so that `etch-bridge` can relay all of this back to etch-core.

## Server Provisioning

Etch uses a custom Matrix state event to tell rooms apart. Without it, every room shows up as a plain text channel.

To mark a room as a voice channel, send an `etch.room_config` state event into it:

```json
{
  "type": "etch.room_config",
  "state_key": "",
  "content": {
    "room_type": "voice",
    "channel_id": 1,
    "is_default": false
  }
}
```

| Field | Description |
|---|---|
| `room_type` | `"voice"` or `"text"` (rooms without this event default to text) |
| `channel_id` | The unique Mumble channel ID to join when the user selects this room. Also determines display order. |
| `is_default` | If `true`, Etch auto-selects this room on connect |

The recommended setup is to create a room mapped to the Mumble root channel (`channel_id: 0`) and mark it as default. This ensures Mumble and Matrix stay in sync on connect.

To tell Etch where the Mumble server is, send an `etch.voice_server` state event on the default room:

```json
{
  "type": "etch.voice_server",
  "state_key": "",
  "content": {
    "host": "mumble.example.com",
    "port": 64738,
    "password": "optional"
  }
}
```

| Field | Description |
|---|---|
| `host` | Mumble server hostname or IP |
| `port` | Mumble server port (defaults to 64738 if omitted) |
| `password` | Server password, if required |

When Etch connects, it reads this from Matrix and uses it to connect to Mumble automatically. Users can still override these values in the bookmark's advanced options.

Both events can be sent with any Matrix client that supports custom state events.
