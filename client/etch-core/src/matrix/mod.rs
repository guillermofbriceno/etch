pub mod client;
pub mod service;
pub mod sync;
pub mod timeline;

pub use sync::sync_loop;
pub use sync::fetch_rooms;
pub use sync::build_room_info;
pub use sync::find_voice_server;

pub use client::send_message;

pub use service::MatrixService;
