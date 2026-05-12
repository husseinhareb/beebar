//! Active-window module.
//!
//! Source backends:
//!   - `"beewm"` — subscribe to the beewm event socket (`beewm-events.sock`
//!     in `$XDG_RUNTIME_DIR`, fallback `/tmp/beewm-events.sock`). The socket
//!     pushes `window>>title\n` events whenever the focused window changes, so
//!     the bar is updated with zero polling overhead.
//!   - `"hyprland"` — shell out to `hyprctl activewindow -j` each tick.
//!   - `"auto"` (default) — prefer the beewm socket when the compositor is
//!     running, otherwise fall back to Hyprland.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::{Module, ModuleChrome, ModuleView};
use crate::renderer::primitives::TextStyle;

const DEFAULT_MAX_LENGTH: usize = 80;
const DEFAULT_EMPTY_LABEL: &str = "";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Auto,
    Beewm,
    Hyprland,
}

impl Source {
    fn from_config(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("beewm") => Self::Beewm,
            Some("hyprland") | Some("hypr") => Self::Hyprland,
            Some("auto") | None => Self::Auto,
            Some(other) => {
                log::warn!(
                    "Unknown window module backend '{}'; using 'auto'",
                    other
                );
                Self::Auto
            }
        }
    }
}

pub struct WindowModule {
    /// Title pushed by the beewm event socket subscriber thread.
    /// `None` means the socket is not connected (beewm not running).
    beewm_title: Arc<Mutex<Option<String>>>,
    /// Last-known Hyprland title, refreshed per tick when used.
    hyprland_title: String,
    max_length: usize,
    empty_label: String,
    chrome: ModuleChrome,
    source: Source,
    /// Keep the subscriber thread handle alive for the module's lifetime.
    _watcher: Option<thread::JoinHandle<()>>,
}

impl WindowModule {
    pub fn new(
        backend: Option<String>,
        max_length: Option<u32>,
        empty_label: Option<String>,
        chrome: ModuleChrome,
    ) -> Self {
        let source = Source::from_config(backend.as_deref());
        let beewm_title = Arc::new(Mutex::new(None));

        let watcher = if matches!(source, Source::Beewm | Source::Auto) {
            Some(spawn_event_socket_watcher(beewm_title.clone()))
        } else {
            None
        };

        Self {
            beewm_title,
            hyprland_title: String::new(),
            max_length: max_length
                .map(|v| v as usize)
                .filter(|v| *v > 0)
                .unwrap_or(DEFAULT_MAX_LENGTH),
            empty_label: empty_label.unwrap_or_else(|| DEFAULT_EMPTY_LABEL.to_string()),
            chrome,
            source,
            _watcher: watcher,
        }
    }

    fn beewm_snapshot(&self) -> Option<String> {
        self.beewm_title.lock().unwrap().clone()
    }

    fn refresh_hyprland(&mut self) {
        self.hyprland_title = read_hyprland_title().unwrap_or_default();
    }

    fn current_title(&self) -> String {
        match self.source {
            Source::Beewm => self.beewm_snapshot().unwrap_or_default(),
            Source::Hyprland => self.hyprland_title.clone(),
            // When connected to beewm the snapshot is Some; fall back to
            // Hyprland only when the socket is not reachable (None).
            Source::Auto => self
                .beewm_snapshot()
                .unwrap_or_else(|| self.hyprland_title.clone()),
        }
    }
}

impl Module for WindowModule {
    fn update(&mut self) {
        // The beewm path is driven by the subscriber thread; nothing to poll.
        // Hyprland still needs a per-tick query because it has no event socket.
        if matches!(self.source, Source::Hyprland | Source::Auto)
            && self.beewm_snapshot().is_none()
        {
            self.refresh_hyprland();
        }
    }

    fn update_interval(&self) -> std::time::Duration {
        self.chrome
            .update_interval
            .unwrap_or(std::time::Duration::from_millis(100))
    }

    fn view(&self) -> ModuleView {
        let title = self.current_title();
        let text = if title.is_empty() {
            self.empty_label.clone()
        } else {
            truncate_chars(&title, self.max_length)
        };
        self.chrome.apply(ModuleView {
            text,
            style: TextStyle::default(),
            ..Default::default()
        })
    }
}

// ─── beewm event-socket subscriber ──────────────────────────────────────────

fn spawn_event_socket_watcher(shared: Arc<Mutex<Option<String>>>) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("beebar-window-events".into())
        .spawn(move || run_event_socket_watcher(shared))
        .expect("failed to spawn window-module event-socket thread")
}

/// Connect (and reconnect) to the beewm event socket, parse `window>>` events,
/// and push the title into `shared`. Clears the title on disconnect so the
/// `Auto` source can fall back to Hyprland while beewm is not running.
fn run_event_socket_watcher(shared: Arc<Mutex<Option<String>>>) {
    let path = event_socket_path();
    loop {
        match UnixStream::connect(&path) {
            Ok(stream) => {
                log::debug!("[window] connected to beewm event socket");
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    match line {
                        Ok(line) => handle_event_line(&line, &shared),
                        Err(error) => {
                            log::debug!("[window] event socket read error: {error}");
                            break;
                        }
                    }
                }
                // Connection closed — clear the cached title so `Auto` falls
                // back to Hyprland until beewm is running again.
                *shared.lock().unwrap() = None;
                log::debug!("[window] disconnected from beewm event socket; will retry");
            }
            Err(_) => {
                // beewm is not running yet; retry shortly.
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn handle_event_line(line: &str, shared: &Arc<Mutex<Option<String>>>) {
    if let Some(title) = line.strip_prefix("window>>") {
        let value = if title.is_empty() {
            Some(String::new())
        } else {
            Some(title.to_string())
        };
        *shared.lock().unwrap() = value;
    }
}

fn event_socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map(|dir| dir.join("beewm-events.sock"))
        .unwrap_or_else(|| PathBuf::from("/tmp/beewm-events.sock"))
}

// ─── Hyprland fallback ───────────────────────────────────────────────────────

fn read_hyprland_title() -> Option<String> {
    let output = Command::new("hyprctl")
        .arg("activewindow")
        .arg("-j")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    extract_title(&raw).filter(|t| !t.is_empty())
}

/// Pull the `"title"` value out of `hyprctl activewindow -j` JSON output
/// without pulling in a full JSON dependency.
fn extract_title(json: &str) -> Option<String> {
    let key = "\"title\"";
    let key_pos = json.find(key)?;
    let after_key = &json[key_pos + key.len()..];
    let colon = after_key.find(':')?;
    let after_colon = &after_key[colon + 1..];
    let quote = after_colon.find('"')?;
    let value_start = quote + 1;
    let bytes = after_colon.as_bytes();
    let mut i = value_start;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            let ch = match next {
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                _ => next as char,
            };
            out.push(ch);
            i += 2;
        } else if b == b'"' {
            return Some(out);
        } else {
            let rest = &after_colon[i..];
            let ch = rest.chars().next()?;
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    None
}

// ─── Utilities ───────────────────────────────────────────────────────────────

fn truncate_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::{Source, extract_title, truncate_chars};

    #[test]
    fn extracts_title_from_hyprctl_json() {
        let json = r#"{"address":"0x1","title":"hello world","class":"Foo"}"#;
        assert_eq!(extract_title(json).as_deref(), Some("hello world"));
    }

    #[test]
    fn handles_escaped_quote_in_title() {
        let json = r#"{"title":"grim -g \"$(slurp)\" s ~"}"#;
        assert_eq!(
            extract_title(json).as_deref(),
            Some("grim -g \"$(slurp)\" s ~")
        );
    }

    #[test]
    fn missing_title_returns_none() {
        let json = r#"{"address":"0x1"}"#;
        assert_eq!(extract_title(json), None);
    }

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate_chars("abcdef", 4), "abc…");
        assert_eq!(truncate_chars("abc", 4), "abc");
    }

    #[test]
    fn source_from_config_parses_known_values() {
        assert_eq!(Source::from_config(None), Source::Auto);
        assert_eq!(Source::from_config(Some("auto")), Source::Auto);
        assert_eq!(Source::from_config(Some("beewm")), Source::Beewm);
        assert_eq!(Source::from_config(Some("hyprland")), Source::Hyprland);
        assert_eq!(Source::from_config(Some("HYPR")), Source::Hyprland);
        assert_eq!(Source::from_config(Some("nope")), Source::Auto);
    }
}
