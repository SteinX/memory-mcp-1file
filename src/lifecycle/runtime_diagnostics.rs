use std::backtrace::Backtrace;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

const RUNTIME_DIR: &str = "runtime";

pub fn install_panic_hook(data_dir: PathBuf, mode: String) {
    let previous_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |panic_info| {
        let payload = panic_payload_to_string(panic_info);
        let location = panic_info
            .location()
            .map(|location| {
                json!({
                    "file": location.file(),
                    "line": location.line(),
                    "column": location.column(),
                })
            })
            .unwrap_or(Value::Null);
        let backtrace = Backtrace::force_capture().to_string();
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed").to_string();

        let details = json!({
            "panic_message": payload,
            "thread": thread_name,
            "location": location,
            "backtrace": backtrace,
        });

        write_runtime_event(&data_dir, "last_panic.json", "panic", &mode, details);
        eprintln!(
            "[memory-mcp] panic captured: mode={} pid={} ppid={} thread={}",
            mode,
            std::process::id(),
            parent_process_id(),
            std::thread::current().name().unwrap_or("unnamed")
        );

        previous_hook(panic_info);
    }));
}

pub fn record_runtime_event(data_dir: &Path, marker_name: &str, event: &str, mode: &str) {
    write_runtime_event(data_dir, marker_name, event, mode, Value::Null);
}

pub fn record_runtime_event_with_details(
    data_dir: &Path,
    marker_name: &str,
    event: &str,
    mode: &str,
    details: Value,
) {
    write_runtime_event(data_dir, marker_name, event, mode, details);
}

pub fn spawn_heartbeat(
    data_dir: PathBuf,
    mode: String,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    write_runtime_event(
                        &data_dir,
                        "last_heartbeat.json",
                        "heartbeat",
                        &mode,
                        Value::Null,
                    );
                }
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
    });
}

fn write_runtime_event(
    data_dir: &Path,
    marker_name: &str,
    event: &str,
    mode: &str,
    details: Value,
) {
    let runtime_dir = data_dir.join(RUNTIME_DIR);
    if let Err(err) = std::fs::create_dir_all(&runtime_dir) {
        eprintln!(
            "[memory-mcp] failed to create runtime diagnostics dir {}: {}",
            runtime_dir.display(),
            err
        );
        return;
    }

    let marker_path = runtime_dir.join(marker_name);
    let payload = json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "event": event,
        "mode": mode,
        "pid": std::process::id(),
        "ppid": parent_process_id(),
        "details": details,
    });

    match serde_json::to_vec_pretty(&payload) {
        Ok(json) => {
            if let Err(err) = std::fs::write(&marker_path, json) {
                eprintln!(
                    "[memory-mcp] failed to write runtime marker {}: {}",
                    marker_path.display(),
                    err
                );
            }
        }
        Err(err) => {
            eprintln!(
                "[memory-mcp] failed to serialize runtime marker {}: {}",
                marker_path.display(),
                err
            );
        }
    }
}

fn panic_payload_to_string(panic_info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(message) = panic_info.payload().downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = panic_info.payload().downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

#[cfg(unix)]
fn parent_process_id() -> u32 {
    unsafe { libc::getppid() as u32 }
}

#[cfg(not(unix))]
fn parent_process_id() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_runtime_marker_file() {
        let temp = tempfile::tempdir().unwrap();

        record_runtime_event_with_details(
            temp.path(),
            "last_start.json",
            "process_start",
            "stdio",
            json!({"test": true}),
        );

        let marker = std::fs::read_to_string(temp.path().join("runtime/last_start.json")).unwrap();
        let parsed: Value = serde_json::from_str(&marker).unwrap();

        assert_eq!(parsed["event"], "process_start");
        assert_eq!(parsed["mode"], "stdio");
        assert_eq!(parsed["details"]["test"], true);
    }
}
