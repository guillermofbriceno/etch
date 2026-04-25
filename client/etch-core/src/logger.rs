use log::{Log, Metadata, Record, Level, LevelFilter};
use simple_logger::SimpleLogger;
use tokio::sync::mpsc;

use crate::events::{CoreEvent, SystemEvent};

pub struct ForwardingLogger {
    inner: SimpleLogger,
    event_tx: mpsc::Sender<CoreEvent>,
}

impl Log for ForwardingLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &Record) {
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

pub fn init(event_tx: mpsc::Sender<CoreEvent>) {
    let inner = SimpleLogger::new()
        .with_level(LevelFilter::Debug)
        .with_module_level("html5ever", LevelFilter::Warn);

    let logger = ForwardingLogger { inner, event_tx };

    log::set_boxed_logger(Box::new(logger))
        .map(|()| log::set_max_level(LevelFilter::Debug))
        .unwrap();
}
