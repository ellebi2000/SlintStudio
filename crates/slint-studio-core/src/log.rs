//! A bounded, thread-safe log buffer backing the "Output" panel.
//!
//! Producers (the tracing layer, the LSP reader thread, the compiler runner)
//! push entries from any thread; the UI polls [`LogBuffer::generation`] once per
//! frame and only takes a [`LogBuffer::snapshot`] when something changed.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

/// Severity of a log entry, ordered from least to most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Fixed-width label suitable for aligned log output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO ",
            Self::Warn => "WARN ",
            Self::Error => "ERROR",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label().trim_end())
    }
}

/// A single line in the Output panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp: SystemTime,
    pub level: LogLevel,
    /// Origin of the message, e.g. a Rust module path such as `slint_studio_lsp`.
    pub target: String,
    pub message: String,
}

impl LogEntry {
    /// Creates an entry stamped with the current time.
    pub fn new(level: LogLevel, target: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            timestamp: SystemTime::now(),
            level,
            target: target.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug)]
struct Inner {
    entries: VecDeque<LogEntry>,
    capacity: usize,
    /// Bumped on every mutation so readers can cheaply detect changes.
    generation: u64,
}

/// Cloneable handle to a shared, bounded log buffer.
///
/// When the buffer is full the oldest entries are discarded.
#[derive(Debug, Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<Inner>>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }
}

impl LogBuffer {
    /// Default number of retained entries.
    pub const DEFAULT_CAPACITY: usize = 5_000;

    /// Creates an empty buffer retaining at most `capacity` entries (minimum 1).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                entries: VecDeque::with_capacity(capacity.min(1024)),
                capacity,
                generation: 0,
            })),
        }
    }

    /// Appends an entry, evicting the oldest one if the buffer is full.
    pub fn push(&self, entry: LogEntry) {
        let mut inner = self.lock();
        if inner.entries.len() == inner.capacity {
            inner.entries.pop_front();
        }
        inner.entries.push_back(entry);
        inner.generation = inner.generation.wrapping_add(1);
    }

    /// Removes all entries.
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.entries.clear();
        inner.generation = inner.generation.wrapping_add(1);
    }

    /// Number of retained entries.
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// Whether the buffer holds no entries.
    pub fn is_empty(&self) -> bool {
        self.lock().entries.is_empty()
    }

    /// Changes whenever the content changes; compare against a cached value
    /// to decide whether a new [`snapshot`](Self::snapshot) is needed.
    pub fn generation(&self) -> u64 {
        self.lock().generation
    }

    /// Copies the retained entries, oldest first.
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.lock().entries.iter().cloned().collect()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave `Inner` in an invalid
        // state (every mutation is a single push/pop/clear), so recover.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(message: &str) -> LogEntry {
        LogEntry::new(LogLevel::Info, "test", message)
    }

    #[test]
    fn new_buffer_is_empty() {
        let buffer = LogBuffer::default();
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert!(buffer.snapshot().is_empty());
    }

    #[test]
    fn push_keeps_insertion_order() {
        let buffer = LogBuffer::new(10);
        buffer.push(entry("a"));
        buffer.push(entry("b"));
        let messages: Vec<_> = buffer.snapshot().into_iter().map(|e| e.message).collect();
        assert_eq!(messages, ["a", "b"]);
    }

    #[test]
    fn oldest_entries_are_evicted_when_full() {
        let buffer = LogBuffer::new(2);
        for message in ["a", "b", "c"] {
            buffer.push(entry(message));
        }
        let messages: Vec<_> = buffer.snapshot().into_iter().map(|e| e.message).collect();
        assert_eq!(messages, ["b", "c"]);
    }

    #[test]
    fn zero_capacity_is_clamped_to_one() {
        let buffer = LogBuffer::new(0);
        buffer.push(entry("a"));
        buffer.push(entry("b"));
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn generation_changes_on_every_mutation() {
        let buffer = LogBuffer::new(1);
        let g0 = buffer.generation();
        buffer.push(entry("a"));
        let g1 = buffer.generation();
        // Eviction still counts as a change even though `len` stays at 1.
        buffer.push(entry("b"));
        let g2 = buffer.generation();
        buffer.clear();
        let g3 = buffer.generation();
        assert!(g0 != g1 && g1 != g2 && g2 != g3);
        assert!(buffer.is_empty());
    }

    #[test]
    fn clones_share_the_same_storage_across_threads() {
        let buffer = LogBuffer::default();
        let producer = buffer.clone();
        std::thread::spawn(move || producer.push(entry("from thread")))
            .join()
            .unwrap();
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn levels_are_ordered_by_severity() {
        assert!(LogLevel::Trace < LogLevel::Debug);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
        assert_eq!(LogLevel::Warn.to_string(), "WARN");
        assert_eq!(LogLevel::Warn.label().len(), LogLevel::Error.label().len());
    }
}
