use log::{Log, Metadata, Record, Level, LevelFilter};
use tokio::sync::mpsc;

use crate::events::{CoreEvent, SystemEvent};

pub struct ForwardingLogger {
    inner: Box<dyn Log>,
    event_tx: mpsc::Sender<CoreEvent>,
}

impl Log for ForwardingLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if record.target().starts_with("html5ever") && record.level() > Level::Warn {
            return;
        }

        self.inner.log(record);

        if record.level() == Level::Error {
            let _ = self.event_tx.try_send(CoreEvent::System(SystemEvent::LogError {
                message: record.args().to_string(),
                target: record.target().to_string(),
            }));
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

fn parse_level(s: &str) -> LevelFilter {
    match s.to_lowercase().as_str() {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        "off" => LevelFilter::Off,
        _ => LevelFilter::Debug,
    }
}

pub fn init(event_tx: mpsc::Sender<CoreEvent>, backend: Box<dyn Log>) {
    let level = std::env::var("ETCH_LOG")
        .map(|s| parse_level(&s))
        .unwrap_or(if cfg!(debug_assertions) { LevelFilter::Debug } else { LevelFilter::Info });

    let logger = ForwardingLogger { inner: backend, event_tx };

    log::set_boxed_logger(Box::new(logger))
        .map(|()| log::set_max_level(level))
        .unwrap();
}

pub fn set_level(level_str: &str) {
    let level = parse_level(level_str);
    log::set_max_level(level);
    log::info!("Log level changed to {}", level);
}
