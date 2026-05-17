# Event Scripts

You can run shell commands in response to chat and voice events by adding entries to `event_scripts` in `settings.json`:

```json
{
  "event_scripts": {
    "new_message": "notify-send \"$ETCH_USER\" \"$ETCH_MESSAGE\"",
    "user_join": "notify-send -a Etch \"$ETCH_USER joined voice\"",
    "user_leave": "notify-send -a Etch \"$ETCH_USER left voice\""
  }
}
```

Each value is passed to `/bin/sh -c`. Environment variables prefixed with `ETCH_` carry event-specific data.

Scripts are debounced at 500ms per event type (rapid-fire messages won't spawn hundreds of processes). A script that runs longer than 60 seconds is killed.

Not available on Windows.

## Events

### `new_message`

Fires when a message arrives in any subscribed room from another user (your own messages are excluded).

| Variable | Description |
|---|---|
| `ETCH_USER` | Display name of the sender (falls back to Matrix ID) |
| `ETCH_MESSAGE` | Plain-text message body |
| `ETCH_ROOM` | Room ID |

### `user_join`

Fires when another user joins your current voice channel (by connecting or moving in).

| Variable | Description |
|---|---|
| `ETCH_USER` | Username of the joining user |

### `user_leave`

Fires when another user leaves your current voice channel (by disconnecting or moving out).

| Variable | Description |
|---|---|
| `ETCH_USER` | Username of the departing user |

## Examples

Desktop notification for messages:

```json
"new_message": "notify-send -a Etch \"$ETCH_USER\" \"$ETCH_MESSAGE\""
```

Log voice activity to a file:

```json
"user_join": "echo \"$(date +%H:%M) $ETCH_USER joined\" >> /tmp/etch-voice.log",
"user_leave": "echo \"$(date +%H:%M) $ETCH_USER left\" >> /tmp/etch-voice.log"
```

## Notes

- Scripts are loaded once at app startup. Editing `event_scripts` in `settings.json` requires restarting the app.
- If a script key is absent or empty, nothing happens for that event.
- Stderr from failed scripts is logged at warn level in the app log.
- Quote your variables (`"$ETCH_MESSAGE"`) to avoid word splitting on messages with spaces.
