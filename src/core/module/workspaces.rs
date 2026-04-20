use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use super::{Module, ModuleChrome, ModuleView, TextSegment};
use crate::core::config::{ModuleConfig, resolve_color};
use crate::core::event::{ClickEvent, MouseButton};
use crate::renderer::color::Color;
use crate::renderer::primitives::TextStyle;

const ACTIVE_WORKSPACE_STATE_PATH: &str = "/tmp/beewm_workspace";
const WORKSPACE_STATE_PATH: &str = "/tmp/beewm_workspaces";
const CONTROL_SOCKET_NAME: &str = "beewm-control.sock";
const CONTROL_SOCKET_FALLBACK: &str = "/tmp/beewm-control.sock";
const ACTIVE_LABEL_COLOR: Color = Color::rgb(0.98, 0.82, 0.53);
const OCCUPIED_LABEL_COLOR: Color = Color::rgb(0.57, 0.84, 0.98);
const EMPTY_LABEL_COLOR: Color = Color::rgb(0.57, 0.60, 0.68);

#[derive(Debug, Clone)]
struct WorkspaceColors {
    active: Color,
    occupied: Color,
    empty: Color,
}

impl WorkspaceColors {
    fn from_config(config: &ModuleConfig) -> Self {
        Self {
            active: resolve_color(
                config.active_color.as_deref(),
                ACTIVE_LABEL_COLOR,
                "module.workspaces.active_color",
            ),
            occupied: resolve_color(
                config.occupied_color.as_deref(),
                OCCUPIED_LABEL_COLOR,
                "module.workspaces.occupied_color",
            ),
            empty: resolve_color(
                config.empty_color.as_deref(),
                EMPTY_LABEL_COLOR,
                "module.workspaces.empty_color",
            ),
        }
    }
}

#[derive(Debug, Clone)]
struct WorkspaceSlot {
    number: u32,
    token: String,
    occupied: bool,
}

#[derive(Debug, Clone)]
struct WorkspaceState {
    active: u32,
    occupied: HashSet<u32>,
}

/// Reads workspace state from beewm and renders clickable numbered indicators.
pub struct WorkspacesModule {
    count: u32,
    labels: Vec<String>,
    chrome: ModuleChrome,
    colors: WorkspaceColors,
    active: u32,
    occupied: HashSet<u32>,
    active_state_path: PathBuf,
    workspace_state_path: PathBuf,
    control_socket_path: PathBuf,
    logged_state_warning: bool,
}

impl WorkspacesModule {
    pub fn new(
        count: u32,
        labels: Vec<String>,
        chrome: ModuleChrome,
        config: &ModuleConfig,
    ) -> Self {
        Self {
            count,
            labels,
            chrome,
            colors: WorkspaceColors::from_config(config),
            active: 1,
            occupied: HashSet::new(),
            active_state_path: PathBuf::from(ACTIVE_WORKSPACE_STATE_PATH),
            workspace_state_path: PathBuf::from(WORKSPACE_STATE_PATH),
            control_socket_path: control_socket_path(),
            logged_state_warning: false,
        }
    }

    fn slots(&self) -> Vec<WorkspaceSlot> {
        (1..=self.count)
            .map(|number| {
                let label = self.workspace_label(number);
                let token = if number == self.count {
                    label.clone()
                } else {
                    format!("{label} ")
                };
                WorkspaceSlot {
                    number,
                    token,
                    occupied: self.occupied.contains(&number),
                }
            })
            .collect()
    }

    fn workspace_label(&self, number: u32) -> String {
        self.labels
            .get(number.saturating_sub(1) as usize)
            .filter(|label| !label.is_empty())
            .cloned()
            .unwrap_or_else(|| workspace_label(number))
    }

    fn refresh_workspace_state(&mut self) {
        if let Some(state) = self.read_workspace_state() {
            self.active = state.active;
            self.occupied = state.occupied;
            self.logged_state_warning = false;
            return;
        }

        if !self.logged_state_warning {
            log::warn!(
                "Workspace occupancy state file {} is unavailable; falling back to active workspace only",
                self.workspace_state_path.display()
            );
            self.logged_state_warning = true;
        }

        self.occupied.clear();
        if let Ok(content) = fs::read_to_string(&self.active_state_path) {
            if let Ok(number) = content.trim().parse::<u32>() {
                if (1..=self.count).contains(&number) {
                    self.active = number;
                }
            }
        }
    }

    fn read_workspace_state(&self) -> Option<WorkspaceState> {
        let content = fs::read_to_string(&self.workspace_state_path).ok()?;
        parse_workspace_state(&content, self.count)
    }

    fn request_workspace_switch(&mut self, number: u32) {
        if number == self.active || !(1..=self.count).contains(&number) {
            return;
        }

        match UnixStream::connect(&self.control_socket_path) {
            Ok(mut stream) => {
                let payload = format!("workspace {number}\n");
                if let Err(error) = stream.write_all(payload.as_bytes()) {
                    log::warn!(
                        "Failed to write workspace switch request to {}: {}",
                        self.control_socket_path.display(),
                        error
                    );
                    return;
                }
                self.active = number;
            }
            Err(error) => {
                log::warn!(
                    "Failed to connect to workspace control socket {}: {}",
                    self.control_socket_path.display(),
                    error
                );
            }
        }
    }

    fn workspace_for_click(&self, event: &ClickEvent) -> Option<u32> {
        let slots = self.slots();
        if slots.is_empty() {
            return None;
        }

        let view = self.view();
        let left_pad = view.padding.0;
        let right_pad = view.padding.1;
        let content_width = (event.module_width - left_pad - right_pad).max(0.0);
        if content_width <= 0.0 {
            return None;
        }

        let rel_x = event.x - left_pad;
        if !(0.0..content_width).contains(&rel_x) {
            return None;
        }

        let total_chars = slots
            .iter()
            .map(|slot| slot.token.chars().count())
            .sum::<usize>() as f64;
        if total_chars <= 0.0 {
            return None;
        }

        let char_width = content_width / total_chars;
        let mut cursor = 0.0;
        for slot in slots {
            let token_width = slot.token.chars().count() as f64 * char_width;
            if rel_x < cursor + token_width {
                return Some(slot.number);
            }
            cursor += token_width;
        }

        None
    }
}

impl Module for WorkspacesModule {
    fn update(&mut self) {
        self.refresh_workspace_state();
    }

    fn view(&self) -> ModuleView {
        let base_style = TextStyle {
            color: self.chrome.foreground.unwrap_or(Color::WHITE),
            ..TextStyle::default()
        };
        let active_style = TextStyle {
            color: self.colors.active,
            ..base_style.clone()
        };
        let occupied_style = TextStyle {
            color: self.colors.occupied,
            ..base_style.clone()
        };
        let empty_style = TextStyle {
            color: self.colors.empty,
            ..base_style.clone()
        };

        let mut text = String::new();
        let mut text_segments = Vec::new();

        for slot in self.slots() {
            text.push_str(&slot.token);
            text_segments.push(TextSegment {
                text: slot.token,
                style: if slot.number == self.active {
                    active_style.clone()
                } else if slot.occupied {
                    occupied_style.clone()
                } else {
                    empty_style.clone()
                },
            });
        }

        self.chrome.apply(ModuleView {
            text,
            text_segments,
            style: base_style,
            ..Default::default()
        })
    }

    fn click(&mut self, event: ClickEvent) {
        if event.button != MouseButton::Left {
            return;
        }

        if let Some(number) = self.workspace_for_click(&event) {
            self.request_workspace_switch(number);
        }
    }
}

fn parse_workspace_state(content: &str, count: u32) -> Option<WorkspaceState> {
    let mut active = None;
    let mut occupied = HashSet::new();

    for line in content.lines() {
        let line = line.trim();
        if let Some(raw_active) = line.strip_prefix("active=") {
            let number = raw_active.trim().parse::<u32>().ok()?;
            if !(1..=count).contains(&number) {
                return None;
            }
            active = Some(number);
        } else if let Some(raw_occupied) = line.strip_prefix("occupied=") {
            for part in raw_occupied
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
            {
                let number = part.parse::<u32>().ok()?;
                if (1..=count).contains(&number) {
                    occupied.insert(number);
                }
            }
        }
    }

    Some(WorkspaceState {
        active: active?,
        occupied,
    })
}

fn workspace_label(number: u32) -> String {
    if number == 10 {
        "0".to_string()
    } else {
        number.to_string()
    }
}

fn control_socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| path.join(CONTROL_SOCKET_NAME))
        .unwrap_or_else(|| PathBuf::from(CONTROL_SOCKET_FALLBACK))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{WorkspacesModule, parse_workspace_state};
    use crate::core::config::ModuleConfig;
    use crate::core::module::Module;
    use crate::core::module::ModuleChrome;

    fn default_module_config() -> ModuleConfig {
        ModuleConfig {
            kind: "workspaces".to_string(),
            format: None,
            command: None,
            foreground: None,
            background: None,
            padding_left: None,
            padding_right: None,
            icon: None,
            icon_unknown: None,
            icon_unavailable: None,
            icon_muted: None,
            icon_low: None,
            icon_medium: None,
            icon_high: None,
            icon_full: None,
            labels: None,
            count: None,
            icon_size: None,
            slider_width: None,
            icon_spacing: None,
            icon_gap: None,
            max_volume: None,
            max_brightness: None,
            backend: None,
            device: None,
            interface: None,
            active_color: None,
            occupied_color: None,
            empty_color: None,
            filled_color: None,
            muted_color: None,
            sensor: None,
            warn_threshold: None,
            critical_threshold: None,
            unavailable_color: None,
            warn_color: None,
            critical_color: None,
            glyph_left: None,
            glyph_right: None,
            glyph_filled: None,
            glyph_empty: None,
        }
    }

    fn default_chrome() -> ModuleChrome {
        ModuleChrome {
            foreground: None,
            background: None,
            padding: (8.0, 8.0),
            icon_spacing: None,
        }
    }

    #[test]
    fn parses_workspace_state_file() {
        let state = parse_workspace_state("active=2\noccupied=1, 2, 5\n", 5).unwrap();

        assert_eq!(state.active, 2);
        assert_eq!(state.occupied, HashSet::from([1, 2, 5]));
    }

    #[test]
    fn renders_plain_workspace_numbers() {
        let mut module =
            WorkspacesModule::new(4, Vec::new(), default_chrome(), &default_module_config());
        module.active = 2;
        module.occupied = HashSet::from([2, 4]);

        let view = module.view();
        let rendered_tokens = view
            .text_segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>();

        assert_eq!(view.text, "1 2 3 4");
        assert_eq!(rendered_tokens, vec!["1 ", "2 ", "3 ", "4"]);
    }

    #[test]
    fn uses_configured_workspace_labels() {
        let mut module = WorkspacesModule::new(
            4,
            vec![
                "α".to_string(),
                "β".to_string(),
                "γ".to_string(),
                "δ".to_string(),
            ],
            default_chrome(),
            &default_module_config(),
        );
        module.active = 1;

        let view = module.view();
        let rendered_tokens = view
            .text_segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>();

        assert_eq!(view.text, "α β γ δ");
        assert_eq!(rendered_tokens, vec!["α ", "β ", "γ ", "δ"]);
    }
}
