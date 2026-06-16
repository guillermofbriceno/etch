use log::{Log, Metadata, Record, Level, LevelFilter};
use tokio::sync::mpsc;

use crate::events::{CoreEvent, SystemEvent};

/// A per-module log level override. Any module whose target starts with `prefix`
/// will be silenced above `max_level`.
pub(crate) struct ModuleFilter {
    pub prefix: &'static str,
    pub max_level: Level,
}

/// Default module-level overrides applied when none are provided.
const DEFAULT_FILTERS: &[ModuleFilter] = &[
    ModuleFilter { prefix: "html5ever", max_level: Level::Warn },
];

pub struct ForwardingLogger {
    inner: Box<dyn Log>,
    event_tx: mpsc::Sender<CoreEvent>,
    module_filters: &'static [ModuleFilter],
}

impl Log for ForwardingLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        for filter in self.module_filters {
            if metadata.target().starts_with(filter.prefix)
                && metadata.level() > filter.max_level
            {
                return false;
            }
        }
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
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

    let logger = ForwardingLogger {
        inner: backend,
        event_tx,
        module_filters: DEFAULT_FILTERS,
    };

    log::set_boxed_logger(Box::new(logger))
        .map(|()| log::set_max_level(level))
        .unwrap();
}

pub fn set_level(level_str: &str) {
    let level = parse_level(level_str);
    log::set_max_level(level);
    log::info!("Log level changed to {}", level);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Records each forwarded log as `(level, target, message)`.
    type LoggedEntries = Arc<Mutex<Vec<(Level, String, String)>>>;

    /// A spy `Log` backend that records what it was asked to log and returns a
    /// fixed value from `enabled`. Lets us assert what `ForwardingLogger`
    /// delegates downstream.
    struct SpyLog {
        enabled: bool,
        logged: LoggedEntries,
    }

    impl Log for SpyLog {
        fn enabled(&self, _: &Metadata) -> bool {
            self.enabled
        }
        fn log(&self, record: &Record) {
            self.logged.lock().unwrap().push((
                record.level(),
                record.target().to_string(),
                record.args().to_string(),
            ));
        }
        fn flush(&self) {}
    }

    fn logger_with(
        inner_enabled: bool,
        filters: &'static [ModuleFilter],
    ) -> (ForwardingLogger, LoggedEntries, mpsc::Receiver<CoreEvent>) {
        let logged = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel(8);
        let logger = ForwardingLogger {
            inner: Box::new(SpyLog { enabled: inner_enabled, logged: logged.clone() }),
            event_tx: tx,
            module_filters: filters,
        };
        (logger, logged, rx)
    }

    fn emit(logger: &ForwardingLogger, level: Level, target: &str, msg: &str) {
        logger.log(
            &Record::builder()
                .level(level)
                .target(target)
                .args(format_args!("{}", msg))
                .build(),
        );
    }

    #[test]
    fn parse_level_maps_known_values_case_insensitively() {
        assert_eq!(parse_level("trace"), LevelFilter::Trace);
        assert_eq!(parse_level("DEBUG"), LevelFilter::Debug);
        assert_eq!(parse_level("Info"), LevelFilter::Info);
        assert_eq!(parse_level("warn"), LevelFilter::Warn);
        assert_eq!(parse_level("ERROR"), LevelFilter::Error);
        assert_eq!(parse_level("off"), LevelFilter::Off);
    }

    #[test]
    fn parse_level_defaults_to_debug_for_unknown() {
        assert_eq!(parse_level("verbose"), LevelFilter::Debug);
        assert_eq!(parse_level(""), LevelFilter::Debug);
    }

    #[test]
    fn error_record_is_forwarded_and_emits_log_error_event() {
        let (logger, logged, mut rx) = logger_with(true, DEFAULT_FILTERS);
        emit(&logger, Level::Error, "etch_core::mymod", "boom");

        // Forwarded to the backend...
        let entries = logged.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, Level::Error);

        // ...and surfaced to the UI as a LogError.
        match rx.try_recv() {
            Ok(CoreEvent::System(SystemEvent::LogError { message, target })) => {
                assert_eq!(message, "boom");
                assert_eq!(target, "etch_core::mymod");
            }
            other => panic!("expected LogError event, got {other:?}"),
        }
    }

    #[test]
    fn non_error_record_is_forwarded_without_event() {
        let (logger, logged, mut rx) = logger_with(true, DEFAULT_FILTERS);
        emit(&logger, Level::Info, "etch_core::mymod", "fyi");

        assert_eq!(logged.lock().unwrap().len(), 1, "should still reach the backend");
        assert!(rx.try_recv().is_err(), "non-error logs must not emit a LogError event");
    }

    #[test]
    fn module_filter_suppresses_verbose_target_entirely() {
        // html5ever is capped at Warn: an Info record is dropped before it
        // reaches the backend or produces any event.
        let (logger, logged, mut rx) = logger_with(true, DEFAULT_FILTERS);
        emit(&logger, Level::Info, "html5ever::tokenizer", "noise");

        assert!(logged.lock().unwrap().is_empty(), "verbose html5ever log must be suppressed");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn module_filter_allows_at_or_above_its_cap() {
        // Warn and Error are at/above the cap and must pass through; an Error
        // still emits its event.
        let (logger, logged, mut rx) = logger_with(true, DEFAULT_FILTERS);
        emit(&logger, Level::Warn, "html5ever::x", "warned");
        emit(&logger, Level::Error, "html5ever::x", "failed");

        assert_eq!(logged.lock().unwrap().len(), 2, "Warn and Error should not be filtered");
        match rx.try_recv() {
            Ok(CoreEvent::System(SystemEvent::LogError { message, .. })) => {
                assert_eq!(message, "failed");
            }
            other => panic!("expected LogError from the html5ever error, got {other:?}"),
        }
    }

    #[test]
    fn enabled_delegates_to_backend_for_unfiltered_target() {
        let meta = Metadata::builder().level(Level::Info).target("etch_core::x").build();

        let (allow, _, _) = logger_with(true, DEFAULT_FILTERS);
        assert!(allow.enabled(&meta), "should defer to a backend that accepts");

        let (deny, _, _) = logger_with(false, DEFAULT_FILTERS);
        assert!(!deny.enabled(&meta), "should defer to a backend that rejects");
    }
}
