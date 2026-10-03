pub mod attachment;
pub(crate) mod compress;
pub mod client;
pub(crate) mod retry;
pub mod service;
pub mod sync;
pub mod timeline;
#[cfg(test)]
pub(crate) mod test_server;

pub use sync::sync_loop;
pub use sync::fetch_rooms;
pub use sync::build_room_info;
pub use sync::find_voice_server;

pub use client::send_message;

pub use service::MatrixService;
