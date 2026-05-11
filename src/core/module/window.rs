//! Active-window module.
//!
//! Two backends:
//!
//! 1. **beewm** — read the focused window's title from `/tmp/beewm_window`
//!    (one line of plain text, no trailing newline required). An inotify
//!    watcher keeps the cached title up-to-date in real time, so the bar
//!    renders the new title on its very next tick.
//! 2. **Hyprland** — shell out to `hyprctl activewindow -j` each tick and
//!    parse the JSON `title` field.
//!
//! Source selection (`backend` config field):
//!   - `"beewm"`     — beewm only.
//!   - `"hyprland"`  — Hyprland only.
//!   - `"auto"` (default) — prefer `/tmp/beewm_window` when present, otherwise
//!     fall back to Hyprland.

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::{Module, ModuleChrome, ModuleView};
use crate::renderer::primitives::TextStyle;

const DEFAULT_MAX_LENGTH: usize = 80;
const DEFAULT_EMPTY_LABEL: &str = "";
const BEEWM_WINDOW_STATE_PATH: &str = "/tmp/beewm_window";

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
    /// Title from the beewm file, kept fresh by the inotify thread.
    beewm_title: Arc<Mutex<Option<String>>>,
    /// Last-known Hyprland title, refreshed per tick when used.
    hyprland_title: String,
    max_length: usize,
    empty_label: String,
    chrome: ModuleChrome,
    source: Source,
    /// Keep the watcher handle alive for the module's lifetime.
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

        // Spawn the inotify watcher whenever the beewm backend can be selected.
        let watcher = if matches!(source, Source::Beewm | Source::Auto) {
            Some(spawn_beewm_watcher(
                PathBuf::from(BEEWM_WINDOW_STATE_PATH),
                beewm_title.clone(),
            ))
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
            Source::Auto => self
                .beewm_snapshot()
                .unwrap_or_else(|| self.hyprland_title.clone()),
        }
    }
}

impl Module for WindowModule {
    fn update(&mut self) {
        // beewm state is pushed in by the watcher thread; nothing to do here.
        // Hyprland still needs a poll since it has no equivalent state file.
        if matches!(self.source, Source::Hyprland | Source::Auto)
            && self.beewm_snapshot().is_none()
        {
            self.refresh_hyprland();
        }
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

fn read_beewm_file(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let trimmed = raw.trim_end_matches(['\n', '\r']).to_string();
    Some(trimmed)
}

/// Spawn a thread that watches `path` via inotify and pushes new contents into
/// `shared`. Handles the common atomic-rename write pattern by also watching
/// the parent directory and re-attaching the watch when the file is replaced.
///
/// If the file does not exist yet (beewm not running, or hasn't written), the
/// thread retries every 500ms until it can attach a watch.
fn spawn_beewm_watcher(path: PathBuf, shared: Arc<Mutex<Option<String>>>) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("beebar-window-inotify".into())
        .spawn(move || run_beewm_watcher(path, shared))
        .expect("failed to spawn window-module inotify thread")
}

fn run_beewm_watcher(path: PathBuf, shared: Arc<Mutex<Option<String>>>) {
    // Seed the cache with the current contents (if any) so the very first
    // frame after startup already shows the title.
    push_update(&path, &shared);

    loop {
        match Inotify::open() {
            Ok(mut inotify) => {
                if let Err(error) = setup_watches(&mut inotify, &path) {
                    log::debug!("[window] inotify setup: {error}");
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
                // Drain events forever. On read error, restart from scratch.
                if let Err(error) = drain_events(&mut inotify, &path, &shared) {
                    log::debug!("[window] inotify drain ended: {error}");
                }
            }
            Err(error) => {
                log::warn!("[window] inotify unavailable ({}); falling back to polling", error);
                // Fall back to a slow poll loop so we still pick up changes.
                loop {
                    thread::sleep(Duration::from_millis(500));
                    push_update(&path, &shared);
                }
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn setup_watches(inotify: &mut Inotify, path: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    // Watch the parent for create/move-in events so we catch atomic replaces.
    inotify.add_watch(
        parent,
        IN_CREATE | IN_MOVED_TO | IN_DELETE | IN_DELETE_SELF,
    )?;
    // Watch the file itself for in-place writes. Tolerate ENOENT — the parent
    // watch will tell us when it appears.
    let _ = inotify.add_watch(path, IN_MODIFY | IN_CLOSE_WRITE | IN_MOVE_SELF);
    Ok(())
}

fn drain_events(
    inotify: &mut Inotify,
    path: &Path,
    shared: &Arc<Mutex<Option<String>>>,
) -> io::Result<()> {
    let target_name = path
        .file_name()
        .map(|n| n.to_owned())
        .unwrap_or_default();
    let mut buf = [0u8; 4096];
    loop {
        let events = inotify.read(&mut buf)?;
        let mut should_update = false;
        let mut should_reattach = false;
        for event in events {
            // Events from the file watch have empty name; events from the
            // parent dir watch have the dirent name.
            if event.name.is_empty() {
                should_update = true;
                if event.mask & (IN_MOVE_SELF | IN_DELETE_SELF) != 0 {
                    should_reattach = true;
                }
            } else if event.name == target_name.as_os_str() {
                should_update = true;
                if event.mask & (IN_CREATE | IN_MOVED_TO) != 0 {
                    should_reattach = true;
                }
            }
        }
        if should_update {
            push_update(path, shared);
        }
        if should_reattach {
            // Re-attach the per-file watch so future in-place edits register.
            let _ = inotify.add_watch(path, IN_MODIFY | IN_CLOSE_WRITE | IN_MOVE_SELF);
        }
    }
}

fn push_update(path: &Path, shared: &Arc<Mutex<Option<String>>>) {
    let new = read_beewm_file(path);
    let mut guard = shared.lock().unwrap();
    *guard = new;
}

// ─── Minimal inotify wrapper (libc-direct, no extra deps) ───────────────────

const IN_MODIFY: u32 = 0x0000_0002;
const IN_MOVE_SELF: u32 = 0x0000_0800;
const IN_CLOSE_WRITE: u32 = 0x0000_0008;
const IN_MOVED_TO: u32 = 0x0000_0080;
const IN_CREATE: u32 = 0x0000_0100;
const IN_DELETE: u32 = 0x0000_0200;
const IN_DELETE_SELF: u32 = 0x0000_0400;
const IN_NONBLOCK: c_int = 0o4000;

struct Inotify {
    fd: c_int,
}

struct InotifyEvent {
    mask: u32,
    name: std::ffi::OsString,
}

impl Inotify {
    fn open() -> io::Result<Self> {
        // SAFETY: inotify_init1 is safe to call; returns -1 on error.
        let fd = unsafe { libc::inotify_init1(IN_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd })
    }

    fn add_watch(&mut self, path: &Path, mask: u32) -> io::Result<c_int> {
        let c_path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        // SAFETY: fd is valid for the lifetime of self; c_path is null-terminated.
        let wd = unsafe { libc::inotify_add_watch(self.fd, c_path.as_ptr(), mask) };
        if wd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(wd)
    }

    /// Block until at least one event is available, then parse all events that
    /// fit into `buf` (which must be ≥ sizeof(inotify_event) + NAME_MAX + 1).
    fn read(&mut self, buf: &mut [u8]) -> io::Result<Vec<InotifyEvent>> {
        // Use poll() to block until readable, since the fd is non-blocking.
        let mut pollfd = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pointer is valid for the duration of the call.
        let rc = unsafe { libc::poll(&mut pollfd, 1, -1) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: buf is a valid mutable byte slice; fd is open.
        let n = unsafe {
            libc::read(
                self.fd,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        let n = n as usize;

        let mut out = Vec::new();
        let header_size = std::mem::size_of::<libc::inotify_event>();
        let mut offset = 0;
        while offset + header_size <= n {
            // SAFETY: we just bounds-checked; buf has enough room for a header.
            let ev: &libc::inotify_event = unsafe {
                &*(buf.as_ptr().add(offset) as *const libc::inotify_event)
            };
            let name_len = ev.len as usize;
            let name_start = offset + header_size;
            let name_end = name_start + name_len;
            if name_end > n {
                break;
            }
            let raw_name = &buf[name_start..name_end];
            // The name field is NUL-padded; strip trailing NULs before the OsString.
            let trimmed_name = raw_name
                .iter()
                .position(|&b| b == 0)
                .map(|pos| &raw_name[..pos])
                .unwrap_or(raw_name);
            let name = std::ffi::OsString::from(
                std::str::from_utf8(trimmed_name).unwrap_or("").to_string(),
            );
            out.push(InotifyEvent {
                mask: ev.mask,
                name,
            });
            offset = name_end;
        }
        Ok(out)
    }
}

impl Drop for Inotify {
    fn drop(&mut self) {
        if self.fd >= 0 {
            // SAFETY: fd is owned by self.
            unsafe { libc::close(self.fd) };
        }
    }
}

/// Pull the `"title"` value out of `hyprctl activewindow -j` JSON output.
///
/// We avoid pulling in a full JSON dep for one field — the output is a flat
/// single-line JSON object with predictable escaping.
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
    use super::{extract_title, truncate_chars, Source};

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
