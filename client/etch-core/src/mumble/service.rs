use tokio::sync::mpsc;
use bridge_types::MumbleCommand as BridgeCommand;
use std::path::PathBuf;

use crate::commands::MumbleCommand;
use crate::error::CoreError;
use crate::events::{CoreEvent, InternalEvent};
use crate::models::VoiceServerConfig;
use crate::mumble::process::MumbleProcess;
use crate::traits::VoiceService;

pub struct MumbleVoiceService {
    process: Option<MumbleProcess>,
    event_tx: mpsc::Sender<CoreEvent>,
    data_dir: PathBuf,
    resource_dir: PathBuf,
}

impl MumbleVoiceService {
    pub fn new(
        event_tx: mpsc::Sender<CoreEvent>,
        data_dir: PathBuf,
        resource_dir: PathBuf,
    ) -> Self {
        Self {
            process: None,
            event_tx,
            data_dir,
            resource_dir,
        }
    }
}

impl VoiceService for MumbleVoiceService {
    async fn launch(
        &mut self,
        creds: VoiceServerConfig,
        internal_tx: mpsc::Sender<InternalEvent>,
        show_gui: bool,
        extra_args: &str,
    ) -> Result<(), CoreError> {
        self.shutdown().await;

        let username = creds.username.as_deref().unwrap_or("unknown");
        let proc = MumbleProcess::spawn(
            &creds.host, creds.port, username, creds.password.as_deref(),
            self.event_tx.clone(), internal_tx, show_gui, extra_args,
            &self.data_dir, &self.resource_dir,
        ).await?;
        log::info!("Mumble launched, bridge socket: {}", proc.sock_name);
        self.process = Some(proc);
        Ok(())
    }

    async fn send_command(&mut self, cmd: MumbleCommand) {
        let Some(ref proc) = self.process else { return };

        let bridge_cmd = match cmd {
            MumbleCommand::SwitchChannel(id) => {
                BridgeCommand::SwitchChannel { channel_id: id as i32 }
            }
            MumbleCommand::MuteSelf(muted) => BridgeCommand::SetMuted { muted },
            MumbleCommand::DeafenSelf(deafened) => BridgeCommand::SetDeafened { deafened },
            MumbleCommand::SetUserVolume { session_id, volume_db } => {
                let volume = 10.0_f32.powf(volume_db / 20.0);
                BridgeCommand::SetUserVolume { session: session_id, volume }
            }
            MumbleCommand::SetTransmissionMode(ref mode_str) => {
                let mode = match mode_str.as_str() {
                    "continuous" => bridge_types::TransmissionMode::Continuous,
                    "push_to_talk" => bridge_types::TransmissionMode::PushToTalk,
                    _ => bridge_types::TransmissionMode::VoiceActivation,
                };
                BridgeCommand::SetTransmissionMode { mode }
            }
            MumbleCommand::SetVadThreshold(value) => {
                BridgeCommand::SetVadThreshold { value }
            }
            MumbleCommand::SetVoiceHold(value) => {
                BridgeCommand::SetVoiceHold { value }
            }
            MumbleCommand::SetUseMumbleSettings(_) => return,
        };
        let _ = proc.cmd_tx.send(bridge_cmd).await;
    }

    async fn shutdown(&mut self) {
        if let Some(ref mut proc) = self.process {
            proc.kill().await;
        }
        self.process = None;
    }
}
