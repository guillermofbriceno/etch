use snafu::Snafu;
use std::path::PathBuf;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum CoreError {
    #[snafu(display("Creating directory '{}'", path.display()))]
    CreateDir { source: std::io::Error, path: PathBuf },

    #[snafu(display("Copying '{}' to '{}'", src.display(), dest.display()))]
    CopyFile { source: std::io::Error, src: PathBuf, dest: PathBuf },

    #[snafu(display("Reading '{}'", path.display()))]
    ReadFile { source: std::io::Error, path: PathBuf },

    #[snafu(display("Writing '{}'", path.display()))]
    WriteFile { source: std::io::Error, path: PathBuf },

    #[snafu(display("Renaming '{}' to '{}'", src.display(), dest.display()))]
    RenameFile { source: std::io::Error, src: PathBuf, dest: PathBuf },

    #[snafu(display("Parsing config '{}'", path.display()))]
    ParseConfig { source: serde_json::Error, path: PathBuf },

    #[snafu(display("{}", message))]
    InvalidConfig { message: String },

    #[snafu(display("Locating plugin at '{}'", path.display()))]
    PluginNotFound { source: std::io::Error, path: PathBuf },

    #[snafu(display("Starting bridge listener"))]
    BridgeStart { source: std::io::Error },

    #[snafu(display("Spawning Mumble binary '{}'", path))]
    MumbleSpawn { source: std::io::Error, path: String },
}
