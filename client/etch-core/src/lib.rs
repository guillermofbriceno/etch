#![recursion_limit = "256"]
pub mod engine;
pub mod commands;
pub mod error;
pub mod events;
pub(crate) mod traits;

mod models;
mod connection;
mod matrix;
mod mumble;
mod logger;
mod settings;

#[cfg(test)]
mod test_mocks;

#[cfg(all(test, feature = "integration-tests"))]
mod integration_tests;

use tokio::sync::mpsc;
use std::path::PathBuf;
use crate::engine::*;

pub type ProductionEngine = CoreEngine<matrix::MatrixService, mumble::service::MumbleVoiceService>;

pub fn init_core(
    data_dir: PathBuf,
    resource_dir: PathBuf,
    cmd_tx: mpsc::Sender<commands::CoreCommand>,
    cmd_rx: mpsc::Receiver<commands::CoreCommand>,
    logger: Box<dyn log::Log>,
) -> (CoreHandle, ProductionEngine) {
    let (event_tx, event_rx) = mpsc::channel(100);

    logger::init(event_tx.clone(), logger);

    log::info!("Etch core start.");
    log::info!("Data directory set to: {:?}", data_dir);
    log::info!("Resource directory set to: {:?}", resource_dir);

    let matrix = matrix::MatrixService::new(event_tx.clone(), data_dir.clone());
    let voice = mumble::service::MumbleVoiceService::new(event_tx.clone(), data_dir.clone(), resource_dir);

    let handle = CoreHandle { cmd_tx, event_rx };
    let engine = CoreEngine::new(cmd_rx, event_tx, matrix, voice, data_dir);

    (handle, engine)
}
