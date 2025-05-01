use std::future::Future;
use tokio::sync::{mpsc, oneshot};

use crate::commands::{MatrixCommand, MumbleCommand, ServerConnectionForm};
use crate::error::CoreError;
use crate::events::InternalEvent;
use crate::models::VoiceServerConfig;

pub trait MatrixBackend: Send {
    fn connect(
        &mut self,
        form: ServerConnectionForm,
        internal_tx: mpsc::Sender<InternalEvent>,
    ) -> impl Future<Output = (bool, Option<VoiceServerConfig>)> + Send;

    fn handle_command(&mut self, cmd: MatrixCommand) -> impl Future<Output = ()> + Send;

    fn resolve_user_profile(
        &self,
        username: &str,
    ) -> impl Future<Output = (Option<String>, Option<String>)> + Send;

    fn spawn_media_fetch(
        &self,
        mxc_url: String,
        respond: oneshot::Sender<Result<Vec<u8>, String>>,
    );

    fn subscribe_to_room(&mut self, room_id: &str) -> impl Future<Output = ()> + Send;

    fn reset(&mut self) -> impl Future<Output = ()> + Send;
}

pub trait VoiceService: Send {
    fn launch(
        &mut self,
        creds: VoiceServerConfig,
        internal_tx: mpsc::Sender<InternalEvent>,
        show_gui: bool,
        extra_args: &str,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn send_command(&mut self, cmd: MumbleCommand) -> impl Future<Output = ()> + Send;

    fn shutdown(&mut self) -> impl Future<Output = ()> + Send;
}
