use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Map, Value};

#[derive(Clone)]
pub struct MetricsRecorder {
    inner: Option<Arc<MetricsInner>>,
}

struct MetricsInner {
    path: PathBuf,
    file: Mutex<File>,
}

impl MetricsRecorder {
    pub fn from_env() -> Self {
        let enabled = std::env::var("MEMORY_MCP_METRICS")
            .map(|value| {
                !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "off" | "no"
                )
            })
            .unwrap_or(true);

        if !enabled {
            return Self::disabled();
        }

        let Some(dir) = std::env::var_os("MEMORY_MCP_METRICS_DIR") else {
            return Self::disabled();
        };

        match Self::new(PathBuf::from(dir)) {
            Ok(recorder) => recorder,
            Err(error) => {
                eprintln!("[memory-mcp] failed to initialize metrics output: {error}");
                Self::disabled()
            }
        }
    }

    pub fn new(dir: impl AsRef<Path>) -> std::io::Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let path = dir.join(format!(
            "memory-mcp-metrics-{}-{}.jsonl",
            timestamp,
            std::process::id()
        ));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        Ok(Self {
            inner: Some(Arc::new(MetricsInner {
                path,
                file: Mutex::new(file),
            })),
        })
    }

    pub fn disabled() -> Self {
        Self { inner: None }
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    pub fn output_path(&self) -> Option<PathBuf> {
        self.inner.as_ref().map(|inner| inner.path.clone())
    }

    pub fn record_event(&self, event: &str, fields: Value) {
        let Some(inner) = &self.inner else {
            return;
        };

        let payload = json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "event": event,
            "pid": std::process::id(),
            "fields": fields,
        });

        let Ok(mut line) = serde_json::to_vec(&payload) else {
            return;
        };
        line.push(b'\n');

        match inner.file.lock() {
            Ok(mut file) => {
                if let Err(error) = file.write_all(&line) {
                    eprintln!("[memory-mcp] failed to write metrics line: {error}");
                }
            }
            Err(error) => {
                eprintln!("[memory-mcp] failed to lock metrics file: {error}");
            }
        }
    }

    pub fn record_duration(&self, event: &str, duration: Duration, fields: Value) {
        let mut object = match fields {
            Value::Object(map) => map,
            other => {
                let mut map = Map::new();
                map.insert("details".to_string(), other);
                map
            }
        };
        object.insert(
            "duration_ms".to_string(),
            Value::from(duration.as_secs_f64() * 1000.0),
        );
        self.record_event(event, Value::Object(object));
    }
}

impl Default for MetricsRecorder {
    fn default() -> Self {
        Self::disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_jsonl_metrics_file() {
        let temp = tempfile::tempdir().unwrap();
        let recorder = MetricsRecorder::new(temp.path()).unwrap();

        recorder.record_duration(
            "test_event",
            Duration::from_millis(12),
            json!({"stage": "unit"}),
        );

        let path = recorder.output_path().unwrap();
        let content = std::fs::read_to_string(path).unwrap();
        let line = content.lines().next().unwrap();
        let parsed: Value = serde_json::from_str(line).unwrap();

        assert_eq!(parsed["event"], "test_event");
        assert_eq!(parsed["fields"]["stage"], "unit");
        assert_eq!(parsed["fields"]["duration_ms"], 12.0);
    }

    #[test]
    fn disabled_recorder_does_not_write() {
        let recorder = MetricsRecorder::disabled();
        recorder.record_event("ignored", json!({"ok": true}));
        assert!(!recorder.is_enabled());
        assert!(recorder.output_path().is_none());
    }
}
