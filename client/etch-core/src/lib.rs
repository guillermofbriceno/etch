#![recursion_limit = "256"]
pub mod engine;
pub mod commands;
pub mod error;
pub mod events;

mod models;
mod connection;
mod matrix;
mod mumble;
mod logger;
mod settings;

use tokio::sync::mpsc;
use std::path::PathBuf;
use crate::engine::*;

pub fn init_core(data_dir: PathBuf, resource_dir: PathBuf) -> (CoreHandle, CoreEngine) {
    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (event_tx, event_rx) = mpsc::channel(100);

    logger::init(event_tx.clone());

    log::info!("Etch core start.");
    log::info!("Data directory set to: {:?}", data_dir);
    log::info!("Resource directory set to: {:?}", resource_dir);

    let handle = CoreHandle { cmd_tx, event_rx };
    let engine = CoreEngine::new(cmd_rx, event_tx, data_dir, resource_dir);

    (handle, engine)
}
