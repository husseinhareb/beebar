//! Screen-capture indicator.
//!
//! Shows a red pill naming the app that is capturing the screen, for as long
//! as it is, and nothing at all otherwise. beewm pushes
//! `screencast>>name1,name2` on its event socket whenever the set of capturing
//! programs changes (empty after `>>` when none are).
//!
//! When an app shares the screen through the desktop portal, the program beewm
//! sees is `xdg-desktop-portal-wlr`, which hands the video on over PipeWire.
//! The real app is whoever reads that PipeWire stream, so while the portal is
//! capturing, a helper thread asks `pw-dump` who is linked to the portal's
//! video node. If that finds nobody, the pill just says "Screen".

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use super::{Module, ModuleChrome, ModuleView, TextSegment};
use crate::renderer::color::Color;
use crate::renderer::primitives::TextStyle;

/// Recording dot, U+25CF BLACK CIRCLE. Present in practically every font.
const DEFAULT_ICON: &str = "\u{25CF}";
/// Shown when the portal is capturing but PipeWire names no consumer.
const FALLBACK_LABEL: &str = "Screen";
/// Deep red that keeps white text readable.
const DEFAULT_BACKGROUND: &str = "#d20f39";
/// Not pure white: the bar repaints `Color::WHITE` text in its own foreground.
const DEFAULT_FOREGROUND: &str = "#fefefe";
/// The dot fades to this on every other tick, which reads as a slow pulse.
const DIM_DOT: &str = "#f5a3b5";
/// How often the PipeWire lookup reruns while the portal is capturing.
const RESOLVE_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Shared {
    /// Capturing programs as beewm reports them.
    capturers: Vec<String>,
    /// Apps reading the portal's stream, from the last PipeWire lookup.
    portal_consumers: Vec<String>,
}

pub struct ScreencastModule {
    shared: Arc<Mutex<Shared>>,
    /// Names currently shown, refreshed in `update`.
    labels: Vec<String>,
    pulse_bright: bool,
    icon: String,
    chrome: ModuleChrome,
    _threads: Vec<thread::JoinHandle<()>>,
}

impl ScreencastModule {
    pub fn new(icon: Option<String>, chrome: ModuleChrome) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let threads = vec![
            spawn("beebar-screencast-events", {
                let shared = shared.clone();
                move || watch_events(shared)
            }),
            spawn("beebar-screencast-pipewire", {
                let shared = shared.clone();
                move || resolve_portal_consumers(shared)
            }),
        ];
        Self {
            shared,
            labels: Vec::new(),
            pulse_bright: true,
            icon: icon
                .filter(|icon| !icon.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_ICON.to_string()),
            chrome,
            _threads: threads,
        }
    }
}

impl Module for ScreencastModule {
    fn update(&mut self) {
        let shared = self.shared.lock().unwrap();
        self.labels = display_labels(&shared.capturers, &shared.portal_consumers);
        drop(shared);
        self.pulse_bright = self.labels.is_empty() || !self.pulse_bright;
    }

    fn update_interval(&self) -> Duration {
        self.chrome
            .update_interval
            .unwrap_or(Duration::from_millis(700))
    }

    fn view(&self) -> ModuleView {
        let Some(first) = self.labels.first() else {
            // Nothing to show: an empty view takes no space in the bar.
            return ModuleView {
                padding: (0.0, 0.0),
                ..Default::default()
            };
        };
        let foreground = self
            .chrome
            .foreground
            .or_else(|| Color::from_hex(DEFAULT_FOREGROUND))
            .unwrap_or(Color::WHITE);
        let dot = if self.pulse_bright {
            foreground
        } else {
            Color::from_hex(DIM_DOT).unwrap_or(foreground)
        };
        let label = match self.labels.len() {
            1 => format!(" {first}"),
            n => format!(" {first} +{}", n - 1),
        };
        let style = |color: Color| TextStyle {
            color,
            ..TextStyle::default()
        };
        let mut view = self.chrome.apply(ModuleView {
            text_segments: vec![
                TextSegment {
                    text: self.icon.clone(),
                    style: style(dot),
                },
                TextSegment {
                    text: label,
                    style: style(foreground),
                },
            ],
            ..Default::default()
        });
        if view.background.is_none() {
            view.background = Color::from_hex(DEFAULT_BACKGROUND);
        }
        view
    }
}

fn spawn(name: &str, body: impl FnOnce() + Send + 'static) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name(name.into())
        .spawn(body)
        .expect("failed to spawn screencast module thread")
}

/// Follow beewm's event socket, reconnecting while beewm is not running.
fn watch_events(shared: Arc<Mutex<Shared>>) {
    let path = super::window::event_socket_path();
    loop {
        if let Ok(stream) = UnixStream::connect(&path) {
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if let Some(names) = parse_event(&line) {
                    shared.lock().unwrap().capturers = names;
                }
            }
            // beewm went away: whatever it reported is no longer true.
            shared.lock().unwrap().capturers.clear();
        }
        thread::sleep(Duration::from_millis(500));
    }
}

/// Re-ask PipeWire who reads the portal's video while the portal is capturing.
fn resolve_portal_consumers(shared: Arc<Mutex<Shared>>) {
    loop {
        let portal_active = shared
            .lock()
            .unwrap()
            .capturers
            .iter()
            .any(|n| is_portal(n));
        let consumers = if portal_active {
            run_pw_dump()
                .map(|json| portal_consumers(&json))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        shared.lock().unwrap().portal_consumers = consumers;
        thread::sleep(if portal_active {
            RESOLVE_INTERVAL
        } else {
            Duration::from_millis(300)
        });
    }
}

fn run_pw_dump() -> Option<String> {
    let output = Command::new("pw-dump")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `screencast>>a,b` to `[a, b]`; `screencast>>` to `[]`; other lines to None.
fn parse_event(line: &str) -> Option<Vec<String>> {
    let names = line.strip_prefix("screencast>>")?;
    Some(
        names
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn is_portal(name: &str) -> bool {
    name.starts_with("xdg-desktop-portal") || name.starts_with("xdpw")
}

/// What the pill lists: direct capturers by name, and in place of the portal
/// the apps reading its stream (or "Screen" when none could be found).
fn display_labels(capturers: &[String], portal_consumers: &[String]) -> Vec<String> {
    let mut labels = BTreeSet::new();
    let mut portal = false;
    for name in capturers {
        if is_portal(name) {
            portal = true;
        } else {
            labels.insert(name.clone());
        }
    }
    if portal {
        if portal_consumers.is_empty() {
            labels.insert(FALLBACK_LABEL.to_string());
        } else {
            labels.extend(portal_consumers.iter().cloned());
        }
    }
    labels.into_iter().collect()
}

/// From `pw-dump` JSON: the apps whose nodes are linked to a video node owned
/// by the screen-cast portal.
fn portal_consumers(json: &str) -> Vec<String> {
    let Ok(Value::Array(objects)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let props = |object: &Value| {
        object
            .pointer("/info/props")
            .cloned()
            .unwrap_or(Value::Null)
    };
    let prop =
        |props: &Value, key: &str| props.get(key).and_then(Value::as_str).map(str::to_string);
    let is_type =
        |object: &Value, kind: &str| object.get("type").and_then(Value::as_str) == Some(kind);

    let nodes: Vec<(u64, Value)> = objects
        .iter()
        .filter(|object| is_type(object, "PipeWire:Interface:Node"))
        .filter_map(|object| Some((object.get("id")?.as_u64()?, props(object))))
        .collect();
    let portal_nodes: Vec<u64> = nodes
        .iter()
        .filter(|(_, props)| {
            [
                "application.process.binary",
                "application.name",
                "node.name",
            ]
            .iter()
            .filter_map(|key| prop(props, key))
            .any(|value| is_portal(&value))
        })
        .map(|(id, _)| *id)
        .collect();

    let mut consumers = BTreeSet::new();
    for link in objects
        .iter()
        .filter(|object| is_type(object, "PipeWire:Interface:Link"))
    {
        let field = |key: &str| {
            link.pointer(&format!("/info/{key}"))
                .and_then(Value::as_u64)
        };
        let (Some(output), Some(input)) = (field("output-node-id"), field("input-node-id")) else {
            continue;
        };
        if !portal_nodes.contains(&output) {
            continue;
        }
        let name = nodes
            .iter()
            .find(|(id, _)| *id == input)
            .and_then(|(_, props)| {
                prop(props, "application.name")
                    .or_else(|| prop(props, "application.process.binary"))
                    .or_else(|| prop(props, "node.name"))
            });
        if let Some(name) = name.filter(|name| !is_portal(name)) {
            consumers.insert(name);
        }
    }
    consumers.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::{display_labels, parse_event, portal_consumers};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn parses_beewm_screencast_events() {
        assert_eq!(parse_event("screencast>>"), Some(vec![]));
        assert_eq!(
            parse_event("screencast>>grim,obs"),
            Some(names(&["grim", "obs"]))
        );
        assert_eq!(parse_event("window>>Firefox"), None);
    }

    #[test]
    fn the_portal_is_replaced_by_the_apps_reading_it() {
        assert!(display_labels(&[], &[]).is_empty());
        assert_eq!(display_labels(&names(&["grim"]), &[]), names(&["grim"]));
        assert_eq!(
            display_labels(&names(&["xdg-desktop-portal-wlr"]), &names(&["Firefox"])),
            names(&["Firefox"])
        );
        assert_eq!(
            display_labels(&names(&["xdg-desktop-portal-wlr"]), &[]),
            names(&["Screen"])
        );
        assert_eq!(
            display_labels(
                &names(&["wf-recorder", "xdg-desktop-portal-wlr"]),
                &names(&["OBS"])
            ),
            names(&["OBS", "wf-recorder"])
        );
    }

    #[test]
    fn finds_the_consumer_of_the_portal_stream_in_pw_dump() {
        let json = r#"[
            {"id": 40, "type": "PipeWire:Interface:Node",
             "info": {"props": {"node.name": "xdpw-stream-1",
                                "application.process.binary": "xdg-desktop-portal-wlr",
                                "media.class": "Video/Source"}}},
            {"id": 41, "type": "PipeWire:Interface:Node",
             "info": {"props": {"node.name": "webrtc-consume-stream",
                                "application.name": "Firefox"}}},
            {"id": 42, "type": "PipeWire:Interface:Node",
             "info": {"props": {"node.name": "alsa_output", "media.class": "Audio/Sink"}}},
            {"id": 50, "type": "PipeWire:Interface:Link",
             "info": {"output-node-id": 40, "input-node-id": 41}},
            {"id": 51, "type": "PipeWire:Interface:Link",
             "info": {"output-node-id": 41, "input-node-id": 42}}
        ]"#;
        assert_eq!(portal_consumers(json), names(&["Firefox"]));
        assert!(portal_consumers("not json").is_empty());
        assert!(portal_consumers("[]").is_empty());
    }
}
