//! Sub-pages of the quick-settings panel, built from system tools:
//!
//! - sound output and input: `pactl` (works with PulseAudio and PipeWire),
//!   every device with its own slider (click its name to make it the
//!   default), then every app playing or recording, like a volume mixer,
//! - Wi-Fi: `nmcli`, nearby networks, click to connect or disconnect,
//! - Bluetooth: `bluetoothctl`, paired devices, click to connect or
//!   disconnect,
//! - notifications: `dunstctl`, recent notifications (click one to show it
//!   again), Do Not Disturb and clearing the history,
//! - custom pages: a list of entries from the config, e.g. a power menu.
//!
//! Each builder returns the rows to draw and, for every row, what clicking
//! it does. Parsing is kept apart from running commands so it can be tested
//! on sample output.

use std::collections::HashMap;

use crate::core::config::PanelMenuItemConfig;
use crate::core::panel::{PageRow, PanelPage};

/// Runs a shell command: (exited with 0, stdout), or `None` on failure to
/// start or timeout.
pub type Runner<'a> = &'a dyn Fn(&str) -> Option<(bool, String)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    AudioOutput,
    AudioInput,
    Wifi,
    Bluetooth,
    Notifications,
    /// Entries from the config, owned by the button with this index.
    Custom(usize),
}

impl PageKind {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "audio-output" | "audio" | "sound" | "volume" => Some(Self::AudioOutput),
            "audio-input" | "microphone" | "mic" => Some(Self::AudioInput),
            "wifi" | "wi-fi" | "network" => Some(Self::Wifi),
            "bluetooth" => Some(Self::Bluetooth),
            "notifications" | "dunst" => Some(Self::Notifications),
            _ => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::AudioOutput => "Sound Output",
            Self::AudioInput => "Sound Input",
            Self::Wifi => "Wi-Fi",
            Self::Bluetooth => "Bluetooth",
            Self::Notifications => "Notifications",
            Self::Custom(_) => "Menu",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    Sink,
    Source,
    SinkInput,
    SourceOutput,
}

impl StreamKind {
    fn pactl_name(self) -> &'static str {
        match self {
            Self::Sink => "sink",
            Self::Source => "source",
            Self::SinkInput => "sink-input",
            Self::SourceOutput => "source-output",
        }
    }
}

/// A device or stream whose volume a page slider controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VolumeTarget {
    pub kind: StreamKind,
    pub index: u64,
}

impl VolumeTarget {
    pub fn set_volume_command(self, percent: u32) -> String {
        format!(
            "pactl set-{}-volume {} {}%",
            self.kind.pactl_name(),
            self.index,
            percent
        )
    }

    pub fn toggle_mute_command(self) -> String {
        format!(
            "pactl set-{}-mute {} toggle",
            self.kind.pactl_name(),
            self.index
        )
    }
}

/// What clicking a page row (or a slider's label line) does.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Click {
    pub command: String,
    /// Close the whole panel (for commands that open a window).
    pub close_panel: bool,
    /// Check this row right away and uncheck the other selectable rows.
    pub select: bool,
    /// Detail text shown on the row while the command runs.
    pub busy_label: Option<String>,
    /// Needs a second click within a few seconds before it runs.
    pub confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RowActions {
    pub click: Option<Click>,
    pub volume: Option<VolumeTarget>,
}

pub struct BuiltPage {
    pub page: PanelPage,
    pub actions: Vec<RowActions>,
}

#[derive(Default)]
struct PageBuilder {
    rows: Vec<PageRow>,
    actions: Vec<RowActions>,
}

impl PageBuilder {
    fn push(&mut self, row: PageRow, actions: RowActions) {
        self.rows.push(row);
        self.actions.push(actions);
    }

    fn section(&mut self, title: &str) {
        self.push(PageRow::Section(title.into()), RowActions::default());
    }

    fn note(&mut self, text: &str) {
        self.push(PageRow::Note(text.into()), RowActions::default());
    }

    fn action(&mut self, icon: &str, label: &str, click: Click) {
        self.push(
            PageRow::Item {
                icon: icon.into(),
                label: label.into(),
                detail: None,
                checked: false,
            },
            RowActions {
                click: Some(click),
                volume: None,
            },
        );
    }

    fn settings(&mut self, label: &str, command: Option<&str>) {
        if let Some(command) = command.filter(|c| !c.trim().is_empty()) {
            self.action(
                ICON_SETTINGS,
                label,
                Click {
                    command: command.into(),
                    close_panel: true,
                    ..Default::default()
                },
            );
        }
    }

    fn finish(self, kind: PageKind) -> BuiltPage {
        self.finish_titled(kind.title())
    }

    fn finish_titled(self, title: &str) -> BuiltPage {
        BuiltPage {
            page: PanelPage {
                title: title.into(),
                rows: self.rows,
            },
            actions: self.actions,
        }
    }
}

// Nerd Font glyphs (Material Design range).
const ICON_SETTINGS: &str = "\u{f0493}";
const ICON_SPEAKER: &str = "\u{f04c3}";
const ICON_HEADPHONES: &str = "\u{f02cb}";
const ICON_MONITOR: &str = "\u{f0379}";
const ICON_MIC: &str = "\u{f036c}";
const ICON_HEADSET: &str = "\u{f02ce}";
const ICON_MUSIC: &str = "\u{f0386}";
const ICON_FIREFOX: &str = "\u{f0239}";
const ICON_CHROME: &str = "\u{f02af}";
const ICON_DISCORD: &str = "\u{f066f}";
const ICON_SPOTIFY: &str = "\u{f04c7}";
const ICON_STEAM: &str = "\u{f04d3}";
const ICON_VIDEO: &str = "\u{f0567}";
const ICON_REFRESH: &str = "\u{f0450}";
const ICON_POWER: &str = "\u{f0425}";
const ICON_BLUETOOTH: &str = "\u{f00af}";
const ICON_GAMEPAD: &str = "\u{f0296}";
const ICON_PHONE: &str = "\u{f03f2}";
const ICON_KEYBOARD: &str = "\u{f030c}";
const ICON_MOUSE: &str = "\u{f037d}";
const WIFI_STRENGTH: [&str; 5] = [
    "\u{f092f}", // nf-md-wifi_strength_outline
    "\u{f091f}", // 1
    "\u{f0922}", // 2
    "\u{f0925}", // 3
    "\u{f0928}", // 4
];

pub fn build(kind: PageKind, settings: Option<&str>, run: Runner) -> BuiltPage {
    match kind {
        PageKind::AudioOutput => build_audio(true, settings, run),
        PageKind::AudioInput => build_audio(false, settings, run),
        PageKind::Wifi => build_wifi(settings, run),
        PageKind::Bluetooth => build_bluetooth(settings, run),
        PageKind::Notifications => build_notifications(settings, run),
        // Built once from the config when opened, see `custom`.
        PageKind::Custom(_) => loading(kind),
    }
}

/// A page listing config entries, e.g. a power menu. Every entry closes the
/// panel when it runs; `confirm` entries need a second click.
pub fn custom(title: &str, items: &[PanelMenuItemConfig]) -> BuiltPage {
    let mut b = PageBuilder::default();
    for item in items {
        b.action(
            &item.icon,
            &item.label,
            Click {
                command: item.command.clone(),
                close_panel: true,
                confirm: item.confirm,
                ..Default::default()
            },
        );
    }
    if items.is_empty() {
        b.note("Nothing configured");
    }
    b.finish_titled(title)
}

/// A page shown until the first real one is built.
pub fn loading(kind: PageKind) -> BuiltPage {
    let mut b = PageBuilder::default();
    b.note("Loading");
    b.finish(kind)
}

/// Single-quote `value` for `sh`.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

// ----------------------------------------------------------------- audio --

#[derive(Debug, Clone, PartialEq)]
struct AudioEntry {
    index: u64,
    name: String,
    label: String,
    volume: f64,
    muted: bool,
    icon: &'static str,
}

fn build_audio(output: bool, settings: Option<&str>, run: Runner) -> BuiltPage {
    let kind = if output {
        PageKind::AudioOutput
    } else {
        PageKind::AudioInput
    };
    let (device_kind, stream_kind) = if output {
        (StreamKind::Sink, StreamKind::SinkInput)
    } else {
        (StreamKind::Source, StreamKind::SourceOutput)
    };
    let devices_json = run(&format!("pactl -f json list {}s", device_kind.pactl_name()));
    let streams_json = run(&format!("pactl -f json list {}s", stream_kind.pactl_name()));
    let default = run(&format!("pactl get-default-{}", device_kind.pactl_name()))
        .map(|(_, out)| out.trim().to_string())
        .unwrap_or_default();

    let mut b = PageBuilder::default();
    let Some(devices) = devices_json
        .filter(|(ok, _)| *ok)
        .and_then(|(_, out)| parse_devices(&out, output))
    else {
        b.note("Could not read audio devices (is pactl installed?)");
        b.settings("Sound Settings", settings);
        return b.finish(kind);
    };

    b.section(if output {
        "Output Devices"
    } else {
        "Input Devices"
    });
    if devices.is_empty() {
        b.note("No devices found");
    }
    for device in devices {
        let is_default = device.name == default;
        b.push(
            PageRow::Slider {
                icon: device.icon.into(),
                label: device.label,
                value: device.volume,
                muted: device.muted,
                checked: Some(is_default),
            },
            RowActions {
                click: Some(Click {
                    command: format!(
                        "pactl set-default-{} {}",
                        device_kind.pactl_name(),
                        shell_quote(&device.name)
                    ),
                    select: true,
                    ..Default::default()
                }),
                volume: Some(VolumeTarget {
                    kind: device_kind,
                    index: device.index,
                }),
            },
        );
    }

    b.section(if output {
        "Playing Audio"
    } else {
        "Recording Audio"
    });
    let streams = streams_json
        .filter(|(ok, _)| *ok)
        .and_then(|(_, out)| parse_streams(&out))
        .unwrap_or_default();
    if streams.is_empty() {
        b.note(if output {
            "No apps are playing audio"
        } else {
            "No apps are recording"
        });
    }
    for stream in streams {
        b.push(
            PageRow::Slider {
                icon: stream.icon.into(),
                label: stream.label,
                value: stream.volume,
                muted: stream.muted,
                checked: None,
            },
            RowActions {
                click: None,
                volume: Some(VolumeTarget {
                    kind: stream_kind,
                    index: stream.index,
                }),
            },
        );
    }

    b.settings("Sound Settings", settings);
    b.finish(kind)
}

/// Average of a pactl `volume` object's channels, as 0.0-1.0.
fn json_volume(entry: &serde_json::Value) -> f64 {
    let Some(channels) = entry.get("volume").and_then(|v| v.as_object()) else {
        return 0.0;
    };
    let values: Vec<f64> = channels
        .values()
        .filter_map(|c| c.get("value_percent")?.as_str())
        .filter_map(|p| p.trim().trim_end_matches('%').trim().parse::<f64>().ok())
        .collect();
    if values.is_empty() {
        return 0.0;
    }
    (values.iter().sum::<f64>() / values.len() as f64 / 100.0).clamp(0.0, 1.0)
}

fn json_str<'a>(entry: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    entry
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn json_prop<'a>(entry: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    json_str(entry.get("properties")?, key)
}

fn parse_devices(json: &str, output: bool) -> Option<Vec<AudioEntry>> {
    let list: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    Some(
        list.iter()
            .filter_map(|d| {
                let name = json_str(d, "name")?.to_string();
                // A source that just mirrors an output is not a microphone.
                let monitor =
                    json_prop(d, "device.class") == Some("monitor") || name.ends_with(".monitor");
                if monitor {
                    return None;
                }
                let label = device_label(d, &name);
                Some(AudioEntry {
                    index: d.get("index")?.as_u64()?,
                    icon: device_icon(&name, json_prop(d, "device.icon_name"), output),
                    name,
                    label,
                    volume: json_volume(d),
                    muted: d.get("mute").and_then(|m| m.as_bool()).unwrap_or(false),
                })
            })
            .collect(),
    )
}

fn parse_streams(json: &str) -> Option<Vec<AudioEntry>> {
    let list: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    Some(
        list.iter()
            .filter_map(|s| {
                // pavucontrol's level meters show up as recording streams.
                if json_prop(s, "application.id") == Some("org.PulseAudio.pavucontrol") {
                    return None;
                }
                let binary = json_prop(s, "application.process.binary").unwrap_or("");
                let app = app_name(json_prop(s, "application.name"), binary);
                let label = match json_prop(s, "media.name") {
                    Some(media) if media != app && !is_generic_media(media) => {
                        format!("{app}: {media}")
                    }
                    _ => app.clone(),
                };
                Some(AudioEntry {
                    index: s.get("index")?.as_u64()?,
                    name: String::new(),
                    icon: app_icon(&format!("{app} {binary}")),
                    label,
                    volume: json_volume(s),
                    muted: s.get("mute").and_then(|m| m.as_bool()).unwrap_or(false),
                })
            })
            .collect(),
    )
}

/// A short, human name for an audio device:
/// - Bluetooth devices keep their own name ("Crusher ANC 2"),
/// - HDMI/DP outputs use the monitor name pactl puts in brackets ("24G4"),
/// - others are "<card> (<port>)", e.g. "Ryzen HD Audio (Line Out)".
fn device_label(device: &serde_json::Value, name: &str) -> String {
    let description = json_str(device, "description").unwrap_or(name);
    if name.starts_with("bluez") {
        return description.to_string();
    }
    if let Some(monitor) = description
        .rsplit_once('[')
        .and_then(|(_, rest)| rest.strip_suffix(']'))
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        return monitor.to_string();
    }

    let card = json_prop(device, "device.product.name").unwrap_or(description);
    let mut card = card.to_string();
    for noise in [
        " Controller",
        " Analog Stereo",
        " Digital Stereo",
        " Stereo",
        " Mono",
    ] {
        card = card.replace(noise, "");
    }
    let card = card.trim();

    let active = json_str(device, "active_port");
    let port = device
        .get("ports")
        .and_then(|p| p.as_array())
        .and_then(|ports| {
            ports
                .iter()
                .find(|p| json_str(p, "name") == active && active.is_some())
        })
        .and_then(|p| json_str(p, "description"));
    match port {
        Some(port) if !card.is_empty() => format!("{card} ({port})"),
        Some(port) => port.to_string(),
        None if !card.is_empty() => card.to_string(),
        None => description.to_string(),
    }
}

/// The app's display name. Browser and Electron voice engines report
/// themselves as "WEBRTC VoiceEngine" and ALSA clients as "ALSA plug-in
/// [prog]", so the program name is clearer for those.
fn app_name(app: Option<&str>, binary: &str) -> String {
    let generic = app.is_none_or(|a| {
        let a = a.to_ascii_lowercase();
        a.contains("webrtc") || a.starts_with("alsa plug-in")
    });
    match (app, binary.is_empty()) {
        (Some(app), _) if !generic => app.to_string(),
        (_, false) => {
            let mut chars = binary.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
        (Some(app), true) => app.to_string(),
        (None, true) => "Unknown app".to_string(),
    }
}

/// Stream titles that say nothing about what is playing.
fn is_generic_media(media: &str) -> bool {
    matches!(
        media.trim().to_ascii_lowercase().as_str(),
        "playstream"
            | "recstream"
            | "playback"
            | "playback stream"
            | "record stream"
            | "audio stream"
            | "audiostream"
            | "audio"
            | "output"
            | "input"
    )
}

fn device_icon(name: &str, icon_name: Option<&str>, output: bool) -> &'static str {
    let hint = format!("{name} {}", icon_name.unwrap_or("")).to_ascii_lowercase();
    if hint.contains("bluez") || hint.contains("headset") || hint.contains("headphone") {
        if output {
            ICON_HEADPHONES
        } else {
            ICON_HEADSET
        }
    } else if !output {
        ICON_MIC
    } else if hint.contains("hdmi") || hint.contains("displayport") {
        ICON_MONITOR
    } else {
        ICON_SPEAKER
    }
}

pub(super) fn app_icon(app: &str) -> &'static str {
    let app = app.to_ascii_lowercase();
    let table: [(&[&str], &'static str); 6] = [
        (&["firefox", "librewolf", "zen"], ICON_FIREFOX),
        (&["chrom", "brave", "vivaldi"], ICON_CHROME),
        (&["discord", "vesktop", "webcord"], ICON_DISCORD),
        (&["spotify"], ICON_SPOTIFY),
        (&["steam"], ICON_STEAM),
        (&["mpv", "vlc", "celluloid"], ICON_VIDEO),
    ];
    table
        .iter()
        .find(|(names, _)| names.iter().any(|n| app.contains(n)))
        .map(|(_, icon)| *icon)
        .unwrap_or(ICON_MUSIC)
}

// ------------------------------------------------------------------ wifi --

#[derive(Debug, Clone, PartialEq)]
struct Network {
    ssid: String,
    signal: u32,
    secured: bool,
    in_use: bool,
}

const MAX_NETWORKS: usize = 8;

fn build_wifi(settings: Option<&str>, run: Runner) -> BuiltPage {
    let mut b = PageBuilder::default();
    let radio = run("nmcli radio wifi");
    let Some((true, state)) = radio else {
        b.note("Could not read Wi-Fi state (is NetworkManager running?)");
        b.settings("Network Settings", settings);
        return b.finish(PageKind::Wifi);
    };
    if state.trim() != "enabled" {
        b.note("Wi-Fi is turned off");
        b.action(
            ICON_POWER,
            "Turn On Wi-Fi",
            Click {
                command: "nmcli radio wifi on".into(),
                ..Default::default()
            },
        );
        b.settings("Network Settings", settings);
        return b.finish(PageKind::Wifi);
    }

    let networks = run("nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi list --rescan no")
        .map(|(_, out)| parse_networks(&out))
        .unwrap_or_default();
    let saved = run("nmcli -t -f NAME,TYPE connection show")
        .map(|(_, out)| parse_connections(&out))
        .unwrap_or_default();
    let active = run("nmcli -t -f NAME,TYPE connection show --active")
        .map(|(_, out)| parse_connections(&out))
        .unwrap_or_default();

    b.section("Networks");
    if networks.is_empty() {
        b.note("No networks found");
    }
    for network in networks.into_iter().take(MAX_NETWORKS) {
        let ssid = shell_quote(&network.ssid);
        let known = saved.contains(&network.ssid);
        let (detail, click) = if network.in_use {
            // Take down whichever profile is active, it may not be named
            // after the network.
            let connection = active.first().cloned().unwrap_or(network.ssid.clone());
            (
                "Connected",
                Click {
                    command: format!("nmcli connection down id {}", shell_quote(&connection)),
                    busy_label: Some("Disconnecting".into()),
                    ..Default::default()
                },
            )
        } else {
            let command = if known {
                format!("nmcli connection up id {ssid}")
            } else {
                // Unknown secured networks ask for the password through the
                // running secret agent (nm-applet and the like).
                format!("nmcli device wifi connect {ssid}")
            };
            let detail = match (known, network.secured) {
                (true, _) => "Saved",
                (false, true) => "Secured",
                (false, false) => "Open",
            };
            (
                detail,
                Click {
                    command,
                    busy_label: Some("Connecting".into()),
                    ..Default::default()
                },
            )
        };
        let strength = ((network.signal.min(100) as usize) * 4)
            .div_ceil(100)
            .min(4);
        b.push(
            PageRow::Item {
                icon: WIFI_STRENGTH[strength].into(),
                label: network.ssid,
                detail: Some(detail.into()),
                checked: network.in_use,
            },
            RowActions {
                click: Some(click),
                volume: None,
            },
        );
    }

    b.section("Options");
    b.action(
        ICON_REFRESH,
        "Scan for Networks",
        Click {
            command: "nmcli device wifi rescan".into(),
            busy_label: Some("Scanning".into()),
            ..Default::default()
        },
    );
    b.action(
        ICON_POWER,
        "Turn Off Wi-Fi",
        Click {
            command: "nmcli radio wifi off".into(),
            ..Default::default()
        },
    );
    b.settings("Network Settings", settings);
    b.finish(PageKind::Wifi)
}

/// Split one line of `nmcli -t` output on unescaped colons.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    fields.last_mut().unwrap().push(next);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

/// Networks by SSID (strongest access point wins), connected first, then by
/// signal strength. Hidden networks are left out.
fn parse_networks(output: &str) -> Vec<Network> {
    let mut by_ssid: HashMap<String, Network> = HashMap::new();
    for line in output.lines() {
        let fields = split_terse(line);
        let [in_use, ssid, signal, security] = fields.as_slice() else {
            continue;
        };
        if ssid.trim().is_empty() {
            continue;
        }
        let network = Network {
            ssid: ssid.clone(),
            signal: signal.trim().parse().unwrap_or(0),
            secured: !security.trim().is_empty() && security.trim() != "--",
            in_use: in_use.trim() == "*",
        };
        by_ssid
            .entry(network.ssid.clone())
            .and_modify(|seen| {
                seen.in_use |= network.in_use;
                seen.signal = seen.signal.max(network.signal);
                seen.secured |= network.secured;
            })
            .or_insert(network);
    }
    let mut networks: Vec<Network> = by_ssid.into_values().collect();
    networks.sort_by(|a, b| {
        b.in_use
            .cmp(&a.in_use)
            .then(b.signal.cmp(&a.signal))
            .then(a.ssid.cmp(&b.ssid))
    });
    networks
}

/// Names of Wi-Fi connection profiles in `nmcli -t -f NAME,TYPE` output.
fn parse_connections(output: &str) -> Vec<String> {
    output
        .lines()
        .map(split_terse)
        .filter(|f| f.len() == 2 && f[1] == "802-11-wireless")
        .map(|f| f[0].clone())
        .collect()
}

// --------------------------------------------------------- notifications --

#[derive(Debug, Clone, PartialEq)]
struct Notification {
    id: u64,
    app: String,
    summary: String,
    body: String,
    /// Microseconds since boot (dunst uses the monotonic clock).
    timestamp_us: u64,
}

const MAX_NOTIFICATIONS: usize = 8;
const ICON_BELL: &str = "\u{f009a}";
const ICON_BELL_OFF: &str = "\u{f009b}";
const ICON_CLEAR: &str = "\u{f05e9}";

fn build_notifications(settings: Option<&str>, run: Runner) -> BuiltPage {
    let mut b = PageBuilder::default();
    let Some(history) = run("dunstctl history")
        .filter(|(ok, _)| *ok)
        .and_then(|(_, out)| parse_dunst_history(&out))
    else {
        b.note("Could not read notifications (is dunst running?)");
        b.settings("Notification Settings", settings);
        return b.finish(PageKind::Notifications);
    };
    let paused = run("dunstctl is-paused").is_some_and(|(_, out)| out.trim() == "true");
    let uptime = run("cat /proc/uptime")
        .and_then(|(_, out)| out.split_whitespace().next()?.parse::<f64>().ok());

    b.section("Recent");
    if history.is_empty() {
        b.note("No notifications");
    }
    for notification in history.iter().take(MAX_NOTIFICATIONS) {
        let label = match (
            notification.summary.is_empty(),
            notification.body.is_empty(),
        ) {
            (false, false) if notification.summary != notification.body => {
                format!("{}: {}", notification.summary, notification.body)
            }
            (false, _) => notification.summary.clone(),
            (true, false) => notification.body.clone(),
            (true, true) => notification.app.clone(),
        };
        let age = uptime.map(|now| age_label(now - notification.timestamp_us as f64 / 1e6));
        b.push(
            PageRow::Item {
                icon: ICON_BELL.into(),
                label,
                detail: age,
                checked: false,
            },
            RowActions {
                // Brings the notification back on screen.
                click: Some(Click {
                    command: format!("dunstctl history-pop {}", notification.id),
                    ..Default::default()
                }),
                volume: None,
            },
        );
    }

    b.section("Options");
    b.push(
        PageRow::Item {
            icon: ICON_BELL_OFF.into(),
            label: "Do Not Disturb".into(),
            detail: None,
            checked: paused,
        },
        RowActions {
            click: Some(Click {
                command: "dunstctl set-paused toggle".into(),
                ..Default::default()
            }),
            volume: None,
        },
    );
    if !history.is_empty() {
        b.action(
            ICON_CLEAR,
            "Clear All",
            Click {
                command: "dunstctl history-clear".into(),
                ..Default::default()
            },
        );
    }
    b.settings("Notification Settings", settings);
    b.finish(PageKind::Notifications)
}

/// Notifications from `dunstctl history` (newest first, as dunst lists
/// them), with Pango markup removed.
fn parse_dunst_history(json: &str) -> Option<Vec<Notification>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let list = value.get("data")?.as_array()?.first()?.as_array()?;
    let field = |n: &serde_json::Value, key: &str| -> Option<serde_json::Value> {
        n.get(key)?.get("data").cloned()
    };
    let text = |n: &serde_json::Value, key: &str| {
        field(n, key)
            .and_then(|v| v.as_str().map(strip_markup))
            .unwrap_or_default()
    };
    Some(
        list.iter()
            .filter_map(|n| {
                Some(Notification {
                    id: field(n, "id")?.as_u64()?,
                    app: text(n, "appname"),
                    summary: text(n, "summary"),
                    body: text(n, "body"),
                    timestamp_us: field(n, "timestamp").and_then(|t| t.as_u64()).unwrap_or(0),
                })
            })
            .collect(),
    )
}

/// Drop `<tags>`, decode the common entities and fold whitespace to single
/// spaces, so a notification fits on one line.
fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "now", "5m", "3h", "2d".
fn age_label(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

// ------------------------------------------------------------- bluetooth --

#[derive(Debug, Clone, PartialEq)]
struct BtDevice {
    mac: String,
    name: String,
    connected: bool,
    icon: &'static str,
    battery: Option<u32>,
}

fn build_bluetooth(settings: Option<&str>, run: Runner) -> BuiltPage {
    let mut b = PageBuilder::default();
    let show = run("bluetoothctl show");
    let Some((true, show)) = show.filter(|(_, out)| out.contains("Controller")) else {
        b.note("No Bluetooth adapter found");
        b.settings("Bluetooth Settings", settings);
        return b.finish(PageKind::Bluetooth);
    };
    if !show.contains("Powered: yes") {
        b.note("Bluetooth is turned off");
        b.action(
            ICON_POWER,
            "Turn On Bluetooth",
            Click {
                command: "bluetoothctl power on".into(),
                ..Default::default()
            },
        );
        b.settings("Bluetooth Settings", settings);
        return b.finish(PageKind::Bluetooth);
    }

    let mut devices: Vec<BtDevice> = run("bluetoothctl devices Paired")
        .map(|(_, out)| parse_paired(&out))
        .unwrap_or_default()
        .into_iter()
        .map(|(mac, name)| {
            let info = run(&format!("bluetoothctl info {mac}"))
                .map(|(_, out)| out)
                .unwrap_or_default();
            parse_bt_info(mac, name, &info)
        })
        .collect();
    devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));

    b.section("Devices");
    if devices.is_empty() {
        b.note("No paired devices");
    }
    for device in devices {
        let detail = match (device.connected, device.battery) {
            (true, Some(battery)) => format!("Connected, {battery}%"),
            (true, None) => "Connected".to_string(),
            (false, _) => String::new(),
        };
        let (verb, busy) = if device.connected {
            ("disconnect", "Disconnecting")
        } else {
            ("connect", "Connecting")
        };
        b.push(
            PageRow::Item {
                icon: device.icon.into(),
                label: device.name,
                detail: Some(detail),
                checked: device.connected,
            },
            RowActions {
                click: Some(Click {
                    command: format!("bluetoothctl {verb} {}", device.mac),
                    busy_label: Some(busy.into()),
                    ..Default::default()
                }),
                volume: None,
            },
        );
    }

    b.section("Options");
    b.action(
        ICON_POWER,
        "Turn Off Bluetooth",
        Click {
            command: "bluetoothctl power off".into(),
            ..Default::default()
        },
    );
    b.settings("Bluetooth Settings", settings);
    b.finish(PageKind::Bluetooth)
}

/// (MAC, name) pairs from `bluetoothctl devices` output.
fn parse_paired(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.trim().splitn(3, ' ');
            if parts.next()? != "Device" {
                return None;
            }
            let mac = parts.next()?.to_string();
            let name = parts.next().unwrap_or(&mac).trim().to_string();
            Some((mac, name))
        })
        .collect()
}

fn parse_bt_info(mac: String, name: String, info: &str) -> BtDevice {
    let field = |key: &str| {
        info.lines()
            .map(str::trim)
            .find_map(|line| line.strip_prefix(key))
            .map(str::trim)
    };
    // "Battery Percentage: 0x50 (80)"
    let battery = field("Battery Percentage:").and_then(|v| {
        let inner = v.split('(').nth(1)?.trim_end_matches(')');
        inner.trim().parse().ok()
    });
    let icon = match field("Icon:").unwrap_or("") {
        i if i.starts_with("audio-head") => ICON_HEADPHONES,
        i if i.starts_with("audio") => ICON_SPEAKER,
        i if i.contains("gaming") => ICON_GAMEPAD,
        i if i.starts_with("phone") => ICON_PHONE,
        i if i.contains("keyboard") => ICON_KEYBOARD,
        i if i.contains("mouse") => ICON_MOUSE,
        _ => ICON_BLUETOOTH,
    };
    BtDevice {
        mac,
        name,
        connected: field("Connected:") == Some("yes"),
        icon,
        battery,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINKS: &str = r#"[
      {"index":59,"name":"alsa_output.pci.analog-stereo","description":"Speakers","mute":false,
       "volume":{"front-left":{"value_percent":"80%"},"front-right":{"value_percent":"90%"}},
       "properties":{"device.icon_name":"audio-card-analog"}},
      {"index":60,"name":"alsa_output.pci.hdmi-stereo","mute":false,"volume":{},
       "description":"Navi 21/23 HDMI/DP Audio Controller Digital Stereo (HDMI 5) [24G4]",
       "properties":{}},
      {"index":61,"name":"alsa_output.pci-2.analog-stereo","mute":false,"volume":{},
       "description":"Ryzen HD Audio Controller Analog Stereo","active_port":"analog-output-lineout",
       "ports":[{"name":"analog-output-lineout","description":"Line Out"}],
       "properties":{"device.product.name":"Ryzen HD Audio Controller"}},
      {"index":7,"name":"bluez_output.8C_0D.1","description":"Crusher ANC 2","mute":true,
       "volume":{"mono":{"value_percent":"63%"}},"properties":{}}
    ]"#;

    const SOURCES: &str = r#"[
      {"index":1,"name":"alsa_output.pci.analog-stereo.monitor","description":"Monitor of Speakers",
       "mute":false,"volume":{},"properties":{"device.class":"monitor"}},
      {"index":2,"name":"alsa_input.pci.analog-stereo","description":"Built-in Mic","mute":false,
       "volume":{"mono":{"value_percent":"100%"}},"properties":{"device.class":"sound"}}
    ]"#;

    const STREAMS: &str = r#"[
      {"index":48341,"mute":false,"volume":{"l":{"value_percent":"100%"}},
       "properties":{"application.name":"Firefox","media.name":"Friends","application.process.binary":"firefox"}},
      {"index":5,"mute":false,"volume":{},
       "properties":{"application.name":"PulseAudio Volume Control","application.id":"org.PulseAudio.pavucontrol"}},
      {"index":6,"mute":true,"volume":{"l":{"value_percent":"40%"}},
       "properties":{"application.name":"mpv","media.name":"mpv"}},
      {"index":7,"mute":false,"volume":{},
       "properties":{"application.name":"WEBRTC VoiceEngine","media.name":"playStream",
                     "application.process.binary":"Discord"}}
    ]"#;

    #[test]
    fn parses_audio_devices_and_skips_monitors() {
        let sinks = parse_devices(SINKS, true).unwrap();
        assert_eq!(sinks.len(), 4);
        assert_eq!(sinks[0].label, "Speakers");
        assert!((sinks[0].volume - 0.85).abs() < 1e-9);
        assert_eq!(sinks[0].icon, ICON_SPEAKER);
        assert_eq!(sinks[1].label, "24G4");
        assert_eq!(sinks[1].icon, ICON_MONITOR);
        assert_eq!(sinks[2].label, "Ryzen HD Audio (Line Out)");
        assert_eq!(sinks[3].label, "Crusher ANC 2");
        assert!(sinks[3].muted);
        assert_eq!(sinks[3].icon, ICON_HEADPHONES);

        let sources = parse_devices(SOURCES, false).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].label, "Built-in Mic");
    }

    #[test]
    fn parses_streams_and_skips_level_meters() {
        let streams = parse_streams(STREAMS).unwrap();
        assert_eq!(streams.len(), 3);
        assert_eq!(streams[0].label, "Firefox: Friends");
        assert_eq!(streams[0].icon, ICON_FIREFOX);
        assert_eq!(streams[1].label, "mpv");
        assert!(streams[1].muted);
        // Voice engines are named after the program, without the stream title.
        assert_eq!(streams[2].label, "Discord");
        assert_eq!(streams[2].icon, ICON_DISCORD);
    }

    #[test]
    fn audio_page_marks_default_and_wires_actions() {
        let run = |cmd: &str| -> Option<(bool, String)> {
            Some(match cmd {
                "pactl -f json list sinks" => (true, SINKS.into()),
                "pactl -f json list sink-inputs" => (true, STREAMS.into()),
                "pactl get-default-sink" => (true, "bluez_output.8C_0D.1\n".into()),
                _ => (false, String::new()),
            })
        };
        let built = build(PageKind::AudioOutput, Some("pavucontrol"), &run);
        let rows = &built.page.rows;
        assert_eq!(built.page.title, "Sound Output");
        assert_eq!(rows.len(), built.actions.len());

        let PageRow::Slider { checked, .. } = &rows[1] else {
            panic!("expected device slider");
        };
        assert_eq!(*checked, Some(false));
        let PageRow::Slider { checked, muted, .. } = &rows[4] else {
            panic!("expected device slider");
        };
        assert_eq!(*checked, Some(true));
        assert!(*muted);

        let actions = &built.actions[1];
        assert_eq!(
            actions.click.as_ref().unwrap().command,
            "pactl set-default-sink 'alsa_output.pci.analog-stereo'"
        );
        let target = actions.volume.unwrap();
        assert_eq!(
            target.set_volume_command(40),
            "pactl set-sink-volume 59 40%"
        );

        // App streams follow, then the settings entry that closes the panel.
        let firefox = built.actions[6].volume.unwrap();
        assert_eq!(
            firefox.toggle_mute_command(),
            "pactl set-sink-input-mute 48341 toggle"
        );
        let settings = built.actions.last().unwrap().click.as_ref().unwrap();
        assert!(settings.close_panel);
        assert_eq!(settings.command, "pavucontrol");
    }

    #[test]
    fn audio_page_reports_missing_pactl() {
        let run = |_: &str| -> Option<(bool, String)> { None };
        let built = build(PageKind::AudioInput, None, &run);
        assert!(matches!(built.page.rows[0], PageRow::Note(_)));
    }

    #[test]
    fn splits_nmcli_terse_output_with_escapes() {
        assert_eq!(
            split_terse(r"*:My\:Net:70:WPA2"),
            vec!["*", "My:Net", "70", "WPA2"]
        );
    }

    #[test]
    fn networks_are_deduplicated_and_sorted() {
        let output = " :eduroam:54:WPA2 802.1X\n*:eduroam:58:WPA2 802.1X\n :Cafe:80:\n :::\n :Home:60:WPA2\n";
        let networks = parse_networks(output);
        let names: Vec<&str> = networks.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["eduroam", "Cafe", "Home"]);
        assert!(networks[0].in_use);
        assert_eq!(networks[0].signal, 58);
        assert!(!networks[1].secured);
    }

    #[test]
    fn wifi_page_offers_connect_and_disconnect() {
        let run = |cmd: &str| -> Option<(bool, String)> {
            Some(match cmd {
                "nmcli radio wifi" => (true, "enabled\n".into()),
                c if c.starts_with("nmcli -t -f IN-USE") => (
                    true,
                    "*:eduroam:58:WPA2\n :Home:70:WPA2\n :It's open:30:\n".into(),
                ),
                "nmcli -t -f NAME,TYPE connection show" => (
                    true,
                    "Home:802-11-wireless\neduroam:802-11-wireless\nlo:loopback\n".into(),
                ),
                "nmcli -t -f NAME,TYPE connection show --active" => {
                    (true, "eduroam:802-11-wireless\n".into())
                }
                _ => (false, String::new()),
            })
        };
        let built = build(PageKind::Wifi, None, &run);
        let command = |i: usize| built.actions[i].click.as_ref().unwrap().command.clone();
        assert_eq!(command(1), "nmcli connection down id 'eduroam'");
        assert_eq!(command(2), "nmcli connection up id 'Home'");
        assert_eq!(command(3), r"nmcli device wifi connect 'It'\''s open'");
        let PageRow::Item {
            checked, detail, ..
        } = &built.page.rows[1]
        else {
            panic!("expected network item");
        };
        assert!(*checked);
        assert_eq!(detail.as_deref(), Some("Connected"));
    }

    #[test]
    fn wifi_page_offers_turning_wifi_on() {
        let run = |_: &str| -> Option<(bool, String)> { Some((true, "disabled\n".into())) };
        let built = build(PageKind::Wifi, None, &run);
        assert_eq!(
            built.actions[1].click.as_ref().unwrap().command,
            "nmcli radio wifi on"
        );
    }

    #[test]
    fn parses_bluetooth_devices() {
        let paired = parse_paired("Device 8C:0D:D9:81:4C:E5 Crusher ANC 2\nnoise\n");
        assert_eq!(
            paired,
            vec![("8C:0D:D9:81:4C:E5".to_string(), "Crusher ANC 2".to_string())]
        );

        let info = "\tName: Crusher ANC 2\n\tIcon: audio-headset\n\tConnected: yes\n\tBattery Percentage: 0x50 (80)\n";
        let device = parse_bt_info("8C".into(), "Crusher ANC 2".into(), info);
        assert!(device.connected);
        assert_eq!(device.battery, Some(80));
        assert_eq!(device.icon, ICON_HEADPHONES);

        let pad = parse_bt_info(
            "40".into(),
            "Pad".into(),
            "Icon: input-gaming\nConnected: no\n",
        );
        assert!(!pad.connected);
        assert_eq!(pad.icon, ICON_GAMEPAD);
    }

    #[test]
    fn bluetooth_page_lists_connected_devices_first() {
        let run = |cmd: &str| -> Option<(bool, String)> {
            Some(match cmd {
                "bluetoothctl show" => (true, "Controller AA\n\tPowered: yes\n".into()),
                "bluetoothctl devices Paired" => {
                    (true, "Device AA:01 Alpha\nDevice BB:02 Beta\n".into())
                }
                "bluetoothctl info BB:02" => (true, "Connected: yes\n".into()),
                _ => (true, "Connected: no\n".into()),
            })
        };
        let built = build(PageKind::Bluetooth, Some("bluetui"), &run);
        let PageRow::Item { label, checked, .. } = &built.page.rows[1] else {
            panic!("expected device item");
        };
        assert_eq!(label, "Beta");
        assert!(*checked);
        assert_eq!(
            built.actions[1].click.as_ref().unwrap().command,
            "bluetoothctl disconnect BB:02"
        );
        assert_eq!(
            built.actions[2].click.as_ref().unwrap().command,
            "bluetoothctl connect AA:01"
        );
    }

    #[test]
    fn page_names_parse() {
        assert_eq!(PageKind::parse("Sound"), Some(PageKind::AudioOutput));
        assert_eq!(PageKind::parse("audio-input"), Some(PageKind::AudioInput));
        assert_eq!(PageKind::parse("wifi"), Some(PageKind::Wifi));
        assert_eq!(PageKind::parse("bluetooth"), Some(PageKind::Bluetooth));
        assert_eq!(PageKind::parse("other"), None);
    }

    const DUNST: &str = r#"{"type":"aa{sv}","data":[[
      {"id":{"type":"i","data":59},"appname":{"type":"s","data":"kitty"},
       "summary":{"type":"s","data":"Claude Code"},
       "body":{"type":"s","data":"<b>Waiting</b>   for\n input &amp; more"},
       "timestamp":{"type":"x","data":90133611237}},
      {"id":{"type":"i","data":58},"appname":{"type":"s","data":"discord"},
       "summary":{"type":"s","data":""},"body":{"type":"s","data":""},
       "timestamp":{"type":"x","data":1000000}}
    ]]}"#;

    #[test]
    fn parses_dunst_history() {
        let list = parse_dunst_history(DUNST).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, 59);
        assert_eq!(list[0].body, "Waiting for input & more");
        assert_eq!(list[0].timestamp_us, 90_133_611_237);
        assert!(parse_dunst_history("not json").is_none());
    }

    #[test]
    fn ages_read_like_a_clock() {
        assert_eq!(age_label(5.0), "now");
        assert_eq!(age_label(175.0), "2m");
        assert_eq!(age_label(7300.0), "2h");
        assert_eq!(age_label(200_000.0), "2d");
        assert_eq!(age_label(-3.0), "now");
    }

    #[test]
    fn notifications_page_lists_history_and_options() {
        let run = |cmd: &str| -> Option<(bool, String)> {
            Some(match cmd {
                "dunstctl history" => (true, DUNST.into()),
                "dunstctl is-paused" => (true, "true\n".into()),
                "cat /proc/uptime" => (true, "90308.63 1343054.56\n".into()),
                _ => (false, String::new()),
            })
        };
        let built = build(PageKind::Notifications, None, &run);
        let PageRow::Item { label, detail, .. } = &built.page.rows[1] else {
            panic!("expected notification");
        };
        assert_eq!(label, "Claude Code: Waiting for input & more");
        assert_eq!(detail.as_deref(), Some("2m"));
        assert_eq!(
            built.actions[1].click.as_ref().unwrap().command,
            "dunstctl history-pop 59"
        );
        // Empty summary and body fall back to the app name.
        let PageRow::Item { label, .. } = &built.page.rows[2] else {
            panic!("expected notification");
        };
        assert_eq!(label, "discord");

        let dnd = built
            .page
            .rows
            .iter()
            .position(|r| matches!(r, PageRow::Item { label, .. } if label == "Do Not Disturb"))
            .unwrap();
        assert!(matches!(
            built.page.rows[dnd],
            PageRow::Item { checked: true, .. }
        ));
        assert_eq!(
            built
                .actions
                .last()
                .unwrap()
                .click
                .as_ref()
                .unwrap()
                .command,
            "dunstctl history-clear"
        );
    }

    #[test]
    fn custom_page_lists_config_entries() {
        let items = vec![
            PanelMenuItemConfig {
                icon: "l".into(),
                label: "Lock".into(),
                command: "beelock".into(),
                confirm: false,
            },
            PanelMenuItemConfig {
                icon: "p".into(),
                label: "Power Off".into(),
                command: "systemctl poweroff".into(),
                confirm: true,
            },
        ];
        let built = custom("Power", &items);
        assert_eq!(built.page.title, "Power");
        assert_eq!(built.page.rows.len(), 2);
        let power = built.actions[1].click.as_ref().unwrap();
        assert!(power.confirm && power.close_panel);
        assert!(!built.actions[0].click.as_ref().unwrap().confirm);
    }
}
