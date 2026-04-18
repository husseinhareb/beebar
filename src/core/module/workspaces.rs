use std::fs;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::SystemTime;

use super::{Module, ModuleView, TextSegment};
use crate::core::event::{ClickEvent, MouseButton};
use crate::renderer::color::Color;
use crate::renderer::primitives::TextStyle;

const ACTIVE_WORKSPACE_STATE_PATH: &str = "/tmp/beewm_workspace";
const CONTROL_SOCKET_NAME: &str = "beewm-control.sock";
const CONTROL_SOCKET_FALLBACK: &str = "/tmp/beewm-control.sock";
const ACTIVE_LABEL_COLOR: Color = Color::rgb(0.98, 0.82, 0.53);

#[derive(Debug, Clone)]
struct WorkspaceSlot {
    number: u32,
    label: String,
    token: String,
}

/// Reads the active workspace index from beewm state and renders clickable
/// numbered workspace indicators.
pub struct WorkspacesModule {
    count: u32,
    active: u32,
    state_path: PathBuf,
    control_socket_path: PathBuf,
    last_modified: Option<SystemTime>,
}

impl WorkspacesModule {
    pub fn new(count: u32) -> Self {
        Self {
            count,
            active: 1,
            state_path: PathBuf::from(ACTIVE_WORKSPACE_STATE_PATH),
            control_socket_path: control_socket_path(),
            last_modified: None,
        }
    }

    fn slots(&self) -> Vec<WorkspaceSlot> {
        (1..=self.count)
            .map(|number| {
                let label = workspace_label(number);
                let token = if number == self.count {
                    format!("[{}]", label)
                } else {
                    format!("[{}] ", label)
                };
                WorkspaceSlot {
                    number,
                    label,
                    token,
                }
            })
            .collect()
    }

    fn refresh_active_workspace(&mut self) {
        let metadata = match fs::metadata(&self.state_path) {
            Ok(metadata) => metadata,
            Err(_) => {
                self.last_modified = None;
                return;
            }
        };

        if let Ok(modified) = metadata.modified() {
            if self.last_modified == Some(modified) {
                return;
            }
            self.last_modified = Some(modified);
        } else {
            self.last_modified = None;
        }

        if let Ok(content) = fs::read_to_string(&self.state_path) {
            if let Ok(number) = content.trim().parse::<u32>() {
                if (1..=self.count).contains(&number) {
                    self.active = number;
                }
            }
        }
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
                self.last_modified = None;
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
        self.refresh_active_workspace();
    }

    fn view(&self) -> ModuleView {
        let base_style = TextStyle::default();
        let active_style = TextStyle {
            color: ACTIVE_LABEL_COLOR,
            ..base_style.clone()
        };

        let mut text = String::new();
        let mut text_segments = Vec::new();

        for slot in self.slots() {
            text.push('[');
            text.push_str(&slot.label);
            text.push(']');
            if slot.number != self.count {
                text.push(' ');
            }

            text_segments.push(TextSegment {
                text: "[".to_string(),
                style: base_style.clone(),
            });
            text_segments.push(TextSegment {
                text: slot.label,
                style: if slot.number == self.active {
                    active_style.clone()
                } else {
                    base_style.clone()
                },
            });
            text_segments.push(TextSegment {
                text: "]".to_string(),
                style: base_style.clone(),
            });
            if slot.number != self.count {
                text_segments.push(TextSegment {
                    text: " ".to_string(),
                    style: base_style.clone(),
                });
            }
        }

        ModuleView {
            text,
            text_segments,
            style: base_style,
            background: None,
            padding: (8.0, 8.0),
            ..Default::default()
        }
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
