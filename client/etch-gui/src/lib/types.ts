// ChatMessage matches the Rust etch-core ChatMessageReceive struct (serde serialization).
// Field mapping to Matrix m.room.message event:
//   id        → event_id ("$..." format)
//   sender    → Matrix user ID ("@user:server")
//   body      → content.body (plain text, always present)
//   html_body → content.formatted_body (only when format = "org.matrix.custom.html")
//   timestamp → origin_server_ts (milliseconds since epoch)
// Matches Rust MediaInfo struct (serde serialization)
export type MediaInfo = {
    mxc_url: string;
    mimetype: string;
    size: number;
    width: number;
    height: number;
    duration: number;
};

export type ChatMessage = {
    id: string;
    sender: string;
    body: string;
    html_body: string | null;
    media: MediaInfo | null;
    timestamp: number;
    // Maps emoji key → list of sender user IDs who reacted.
    reactions: Record<string, string[]>;
};

// Matches Rust StateEventKind enum (serde externally tagged)
export type StateEventKind =
    | { RoomNameChanged: { name: string } }
    | { RoomTopicChanged: { topic: string } }
    | { RoomAvatarChanged: { url: string | null } }
    | { MemberJoined: { user_id: string } }
    | { MemberLeft: { user_id: string } }
    | { MemberInvited: { user_id: string } }
    | { MemberBanned: { user_id: string } }
    | 'Other';

// Matches Rust SenderProfile struct (serde serialization)
export type SenderProfile = {
    display_name: string | null;
    avatar_url: string | null;
};

// Matches Rust TimelineEntryKind enum (serde externally tagged).
// Unit variants serialize as plain strings, tuple/struct variants as { Variant: data }.
export type TimelineEntryKind =
    | { Message: ChatMessage }
    | { StateEvent: StateEventKind }
    | { DayDivider: number }
    | 'ReadMarker'
    | 'Redacted'
    | 'Other';

// Matches Rust TimelineEntry struct (serde serialization)
export type TimelineEntry = {
    sender: SenderProfile | null;
    kind: TimelineEntryKind;
};

export type ServerBookmark = {
    id: string;          // unique identifier (crypto.randomUUID())
    label: string;       // display name
    address: string;     // server hostname/IP
    port: number;        // server port
    username: string;    // login username
    auto_connect: boolean; // connect on app startup
    mumble_host: string | null;
    mumble_port: number | null;
    mumble_username: string | null;
    mumble_password: string | null;
};

// Matches Rust RoomType enum (serde serialization)
export type RoomType = 'Voice' | 'Text' | 'Dm';

// Matches Rust RoomInfo struct (serde serialization)
export type RoomInfo = {
    id: string;
    display_name: string;
    etch_room_type: RoomType;
    channel_id: number | null;
    is_default: boolean;
    unread_count: number;
    is_encrypted: boolean;
};
