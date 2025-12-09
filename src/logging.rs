//! Log capture system for streaming logs to the web admin panel.
//!
//! This module provides a custom tracing layer that captures log events
//! and makes them available for streaming via SSE to the admin interface.

use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// A single log entry
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub level: String,
    pub target: String,
    pub message: String,
}

impl LogEntry {
    /// Format as a string for display
    pub fn format(&self) -> String {
        format!(
            "{} {} [{}] {}",
            self.timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            self.level,
            self.target,
            self.message
        )
    }

    /// Format as JSON for SSE
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "timestamp": self.timestamp.to_rfc3339(),
            "level": self.level,
            "target": self.target,
            "message": self.message
        })
        .to_string()
    }
}

/// Buffer that stores recent log entries and broadcasts new ones
pub struct LogBuffer {
    /// Broadcast sender for new log entries
    tx: broadcast::Sender<LogEntry>,
    /// Recent log entries (ring buffer)
    recent: parking_lot::RwLock<Vec<LogEntry>>,
    /// Maximum entries to keep in memory
    max_entries: usize,
}

impl LogBuffer {
    /// Create a new log buffer
    pub fn new(max_entries: usize) -> Self {
        let (tx, _) = broadcast::channel(1000);
        Self {
            tx,
            recent: parking_lot::RwLock::new(Vec::with_capacity(max_entries)),
            max_entries,
        }
    }

    /// Add a log entry
    pub fn push(&self, entry: LogEntry) {
        // Add to recent buffer
        {
            let mut recent = self.recent.write();
            if recent.len() >= self.max_entries {
                recent.remove(0);
            }
            recent.push(entry.clone());
        }

        // Broadcast to subscribers (ignore if no receivers)
        let _ = self.tx.send(entry);
    }

    /// Get recent log entries
    pub fn get_recent(&self, count: usize) -> Vec<LogEntry> {
        let recent = self.recent.read();
        let start = recent.len().saturating_sub(count);
        recent[start..].to_vec()
    }

    /// Subscribe to new log entries
    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.tx.subscribe()
    }
}

/// Shared log buffer type
pub type SharedLogBuffer = Arc<LogBuffer>;

/// Create a shared log buffer
pub fn create_log_buffer(max_entries: usize) -> SharedLogBuffer {
    Arc::new(LogBuffer::new(max_entries))
}

/// Tracing layer that captures logs to the buffer
pub struct LogCaptureLayer {
    buffer: SharedLogBuffer,
}

impl LogCaptureLayer {
    pub fn new(buffer: SharedLogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for LogCaptureLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // Extract the message from the event
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let entry = LogEntry {
            timestamp: chrono::Utc::now(),
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            message: visitor.message,
        };

        self.buffer.push(entry);
    }
}

/// Visitor to extract message from tracing events
#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        } else if self.message.is_empty() {
            // Fallback: use any field if no message field
            if !self.message.is_empty() {
                self.message.push_str(", ");
            }
            self.message.push_str(&format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else if self.message.is_empty() {
            self.message = format!("{}={}", field.name(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer() {
        let buffer = create_log_buffer(3);

        buffer.push(LogEntry {
            timestamp: chrono::Utc::now(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "Message 1".to_string(),
        });

        buffer.push(LogEntry {
            timestamp: chrono::Utc::now(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "Message 2".to_string(),
        });

        let recent = buffer.get_recent(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].message, "Message 1");
        assert_eq!(recent[1].message, "Message 2");
    }

    #[test]
    fn test_log_buffer_overflow() {
        let buffer = create_log_buffer(2);

        for i in 1..=5 {
            buffer.push(LogEntry {
                timestamp: chrono::Utc::now(),
                level: "INFO".to_string(),
                target: "test".to_string(),
                message: format!("Message {}", i),
            });
        }

        let recent = buffer.get_recent(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].message, "Message 4");
        assert_eq!(recent[1].message, "Message 5");
    }
}
