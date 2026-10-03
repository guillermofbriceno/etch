#![recursion_limit = "256"]
pub mod engine;
pub mod commands;
pub mod error;
pub mod events;
pub(crate) mod traits;

mod models;
mod actor;
mod connection;
mod matrix;
mod mumble;
mod logger;
mod task;
pub mod settings;
pub mod temp_uploads;
pub(crate) mod scripting;

#[cfg(test)]
mod test_mocks;

#[cfg(all(test, feature = "integration-tests"))]
mod integration_tests;

use tokio::sync::mpsc;
use std::path::PathBuf;
use std::sync::Arc;
use crate::engine::*;

/// Alias kept for callers that name the type.
pub type ProductionEngine = CoreEngine;

/// Must be called inside a Tokio runtime: the engine spawns its subsystem tasks on construction.
pub fn init_core(
    data_dir: PathBuf,
    resource_dir: PathBuf,
    cmd_tx: mpsc::Sender<commands::CoreCommand>,
    cmd_rx: mpsc::Receiver<commands::CoreCommand>,
    media_rx: mpsc::Receiver<commands::MediaRequest>,
    temp_uploads: temp_uploads::TempUploads,
    logger: Box<dyn log::Log>,
) -> (CoreHandle, ProductionEngine) {
    let (event_tx, event_rx) = mpsc::channel(100);

    logger::init(event_tx.clone(), logger);

    log::info!("Etch core start.");
    log::info!("Data directory set to: {:?}", data_dir);
    log::info!("Resource directory set to: {:?}", resource_dir);

    let settings = settings::load(&data_dir);
    let dispatcher = Arc::new(scripting::ScriptDispatcher::from_settings(&settings));

    let matrix = matrix::MatrixService::new(event_tx.clone(), data_dir.clone(), dispatcher.clone(), temp_uploads);
    let voice = mumble::service::MumbleVoiceService::new(event_tx.clone(), data_dir.clone(), resource_dir, dispatcher);

    let handle = CoreHandle { cmd_tx, event_rx };
    let engine = CoreEngine::new(cmd_rx, media_rx, event_tx, matrix, voice, data_dir, settings);

    (handle, engine)
}
