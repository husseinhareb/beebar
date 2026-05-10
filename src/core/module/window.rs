//! Active-window module.
//!
//! Two backends:
//!
//! 1. **beewm** — read the focused window's title from `/tmp/beewm_window`
//!    (one line of plain text, no trailing newline required). Mirrors the
//!    workspaces module, which reads `/tmp/beewm_workspace`.
//! 2. **Hyprland** — shell out to `hyprctl activewindow -j` and parse the
//!    JSON `title` field.
//!
//! Source selection (`source` config field):
//!   - `"beewm"`     — beewm only.
//!   - `"hyprland"`  — Hyprland only.
//!   - `"auto"` (default) — prefer `/tmp/beewm_window` when present, otherwise
//!     fall back to Hyprland.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
                    "Unknown window module source '{}'; using 'auto'",
                    other
                );
                Self::Auto
            }
        }
    }
}

pub struct WindowModule {
    title: String,
    max_length: usize,
    empty_label: String,
    chrome: ModuleChrome,
    source: Source,
    beewm_path: PathBuf,
}

impl WindowModule {
    pub fn new(
        source: Option<String>,
        max_length: Option<u32>,
        empty_label: Option<String>,
        chrome: ModuleChrome,
    ) -> Self {
        Self {
            title: String::new(),
            max_length: max_length
                .map(|v| v as usize)
                .filter(|v| *v > 0)
                .unwrap_or(DEFAULT_MAX_LENGTH),
            empty_label: empty_label.unwrap_or_else(|| DEFAULT_EMPTY_LABEL.to_string()),
            chrome,
            source: Source::from_config(source.as_deref()),
            beewm_path: PathBuf::from(BEEWM_WINDOW_STATE_PATH),
        }
    }

    fn refresh(&mut self) {
        self.title = match self.source {
            Source::Beewm => self.read_beewm().unwrap_or_default(),
            Source::Hyprland => self.read_hyprland().unwrap_or_default(),
            Source::Auto => self
                .read_beewm()
                .or_else(|| self.read_hyprland())
                .unwrap_or_default(),
        };
    }

    fn read_beewm(&self) -> Option<String> {
        let raw = fs::read_to_string(&self.beewm_path).ok()?;
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn read_hyprland(&self) -> Option<String> {
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
}

impl Module for WindowModule {
    fn update(&mut self) {
        self.refresh();
    }

    fn view(&self) -> ModuleView {
        let text = if self.title.is_empty() {
            self.empty_label.clone()
        } else {
            truncate_chars(&self.title, self.max_length)
        };
        self.chrome.apply(ModuleView {
            text,
            style: TextStyle::default(),
            ..Default::default()
        })
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
            // Standard JSON escapes; just append the escaped char.
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
            // Take the next UTF-8 codepoint.
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
        // Unknown falls back to Auto.
        assert_eq!(Source::from_config(Some("nope")), Source::Auto);
    }
}
