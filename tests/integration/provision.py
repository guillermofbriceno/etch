#!/usr/bin/env python3
"""Provision a Continuwuity instance for integration tests.

Registers test users, creates rooms with etch state events, and joins
all users to all rooms. Designed to run against a fresh, ephemeral
container -- idempotent so it can also run against an existing one.

Continuwuity generates a one-time bootstrap token on first startup.
The configured REGISTRATION_TOKEN only becomes active after the first
user is registered with the bootstrap token. This script scrapes the
bootstrap token from Docker logs to handle that initial registration.
"""

import re
import subprocess
import sys
import time

import requests

BASE_URL = "http://localhost:6167"
SERVER_NAME = "localhost"
REGISTRATION_TOKEN = "devtoken"
MATRIX_CONTAINER = "etch-test-matrix"
MUMBLE_HOST = "localhost"
MUMBLE_PORT = 64738
MUMBLE_PASSWORD = "testpass"

USERS = [
    ("admin", "admin_password"),
    ("alice", "alice_password"),
    ("bob", "bob_password"),
]

ROOMS = [
    {"name": "Lobby",                "type": "voice", "channel_id": 0, "is_default": True},
    {"name": "General Discussion 1", "type": "voice", "channel_id": 1},
    {"name": "General Discussion 2", "type": "voice", "channel_id": 2},
    {"name": "Multimedia Room",      "type": "voice", "channel_id": 3},
    {"name": "Test Text"},
    {"name": "Plaintext Room"},
    {"name": "Encrypted Room", "encrypted": True},
]


def wait_for_server(url, timeout_secs=60):
    """Poll until the Matrix server responds to version queries."""
    deadline = time.time() + timeout_secs
    while time.time() < deadline:
        try:
            r = requests.get(f"{url}/_matrix/client/versions", timeout=3)
            if r.status_code == 200:
                print(f"[*] Server ready at {url}")
                return
        except requests.ConnectionError:
            pass
        time.sleep(1)
    print(f"[!] Server at {url} not ready after {timeout_secs}s", file=sys.stderr)
    sys.exit(1)


def get_bootstrap_token(timeout_secs=30):
    """Scrape the one-time bootstrap token from Continuwuity container logs.

    Continuwuity logs the bootstrap token on first startup. The HTTP endpoint
    may become available before the log line is flushed, so we poll.
    """
    deadline = time.time() + timeout_secs
    while time.time() < deadline:
        result = subprocess.run(
            ["docker", "logs", MATRIX_CONTAINER],
            capture_output=True, text=True,
        )
        # The log line looks like: "...using the registration token <TOKEN> ."
        # ANSI escape codes surround the token, so strip them.
        clean = re.sub(r"\x1b\[[0-9;]*m", "", result.stdout + result.stderr)
        match = re.search(r"using the registration token\s+(\S+)", clean)
        if match:
            return match.group(1)
        time.sleep(1)
    return None


def register_user(username, password, token=None):
    token = token or REGISTRATION_TOKEN
    url = f"{BASE_URL}/_matrix/client/v3/register"
    payload = {
        "username": username,
        "password": password,
        "auth": {"type": "m.login.registration_token", "token": token},
    }
    r = requests.post(url, json=payload)
    if r.status_code == 400 and "M_USER_IN_USE" in r.text:
        print(f"    User {username} already exists, logging in instead.")
        return login_user(username, password)
    r.raise_for_status()
    print(f"    Registered {username}.")
    return r.json()["access_token"]


def login_user(username, password):
    url = f"{BASE_URL}/_matrix/client/v3/login"
    payload = {
        "type": "m.login.password",
        "identifier": {"type": "m.id.user", "user": username},
        "password": password,
    }
    r = requests.post(url, json=payload)
    r.raise_for_status()
    return r.json()["access_token"]


def create_room(token, name, initial_state=None):
    url = f"{BASE_URL}/_matrix/client/v3/createRoom"
    headers = {"Authorization": f"Bearer {token}"}
    payload = {"name": name, "preset": "public_chat"}
    if initial_state:
        payload["initial_state"] = initial_state
    r = requests.post(url, json=payload, headers=headers)
    r.raise_for_status()
    room_id = r.json()["room_id"]
    print(f"    Created room '{name}' -> {room_id}")
    return room_id


def invite_user(token, room_id, user_id):
    url = f"{BASE_URL}/_matrix/client/v3/rooms/{room_id}/invite"
    headers = {"Authorization": f"Bearer {token}"}
    r = requests.post(url, json={"user_id": user_id}, headers=headers)
    r.raise_for_status()


def join_room(token, room_id):
    url = f"{BASE_URL}/_matrix/client/v3/join/{room_id}"
    headers = {"Authorization": f"Bearer {token}"}
    r = requests.post(url, headers=headers)
    r.raise_for_status()


def send_message(token, room_id, body):
    url = f"{BASE_URL}/_matrix/client/v3/rooms/{room_id}/send/m.room.message/{int(time.time() * 1000000)}"
    headers = {"Authorization": f"Bearer {token}"}
    payload = {"msgtype": "m.text", "body": body}
    r = requests.put(url, json=payload, headers=headers)
    r.raise_for_status()


def seed_messages(token, room_id, count):
    """Send `count` messages to a room for pagination testing."""
    print(f"    Seeding {count} messages...")
    for i in range(count):
        send_message(token, room_id, f"seed-msg-{i:04d}")
    print(f"    Done seeding {count} messages.")


def build_initial_state(room_def):
    """Build Matrix initial_state events from a room definition."""
    state = []

    if "channel_id" in room_def:
        state.append({
            "type": "etch.room_config",
            "state_key": "",
            "content": {
                "room_type": room_def.get("type", "text"),
                "channel_id": room_def["channel_id"],
                "is_default": room_def.get("is_default", False),
            },
        })

    if room_def.get("is_default"):
        state.append({
            "type": "etch.voice_server",
            "state_key": "",
            "content": {
                "host": MUMBLE_HOST,
                "port": MUMBLE_PORT,
                "password": MUMBLE_PASSWORD,
            },
        })

    if room_def.get("encrypted"):
        state.append({
            "type": "m.room.encryption",
            "state_key": "",
            "content": {
                "algorithm": "m.megolm.v1.aes-sha2",
            },
        })

    return state or None


def main():
    print("[*] Waiting for Matrix server...")
    wait_for_server(BASE_URL)

    # Continuwuity requires the first user to be registered with a one-time
    # bootstrap token. After that, the configured REGISTRATION_TOKEN works.
    print("[*] Fetching bootstrap token from container logs...")
    bootstrap_token = get_bootstrap_token()
    if bootstrap_token:
        print(f"    Got bootstrap token: {bootstrap_token}")
    else:
        print("    No bootstrap token found (server may already be provisioned).")

    print("[*] Registering users...")
    tokens = {}
    for i, (username, password) in enumerate(USERS):
        # Use bootstrap token for the first user only.
        token = bootstrap_token if i == 0 and bootstrap_token else REGISTRATION_TOKEN
        tokens[username] = register_user(username, password, token=token)

    admin_token = tokens["admin"]
    non_admin = [(u, p) for u, p in USERS if u != "admin"]

    print("[*] Creating rooms...")
    room_ids = []
    for room_def in ROOMS:
        state = build_initial_state(room_def)
        rid = create_room(admin_token, room_def["name"], initial_state=state)
        room_ids.append(rid)

    print("[*] Inviting and joining users...")
    for rid in room_ids:
        for username, _ in non_admin:
            user_id = f"@{username}:{SERVER_NAME}"
            invite_user(admin_token, rid, user_id)
            join_room(tokens[username], rid)

    # Seed history for pagination testing in "Test Text" room.
    test_text_idx = next(i for i, r in enumerate(ROOMS) if r["name"] == "Test Text")
    print("[*] Seeding message history for pagination tests...")
    seed_messages(admin_token, room_ids[test_text_idx], count=500)

    print("[*] Provisioning complete.")


if __name__ == "__main__":
    main()
