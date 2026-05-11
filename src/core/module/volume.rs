use std::env;
use std::process::Command;
use std::time::{Duration, Instant};

use super::{
    Module, ModuleChrome, ModuleView, SliderGlyphs, TextSegment, char_len, pad_text_right,
    prefix_text,
};
use crate::core::config::{ModuleConfig, resolve_color};
use crate::core::event::{ClickEvent, MouseButton};
use crate::renderer::color::Color;
use crate::renderer::primitives::TextStyle;

const DEFAULT_DEVICE_WPCTL: &str = "@DEFAULT_AUDIO_SINK@";
const DEFAULT_DEVICE_PACTL: &str = "@DEFAULT_SINK@";
const DEFAULT_SOURCE_WPCTL: &str = "@DEFAULT_AUDIO_SOURCE@";
const DEFAULT_SOURCE_PACTL: &str = "@DEFAULT_SOURCE@";

/// Whether a `VolumeModule` operates on the default sink (speakers) or
/// the default source (microphone). Selected via the `target` config field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Sink,
    Source,
}

impl Target {
    fn from_config(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("source") | Some("input") | Some("microphone") | Some("mic") => Self::Source,
            _ => Self::Sink,
        }
    }
}
const DEFAULT_SLIDER_WIDTH: usize = 20;
const DEFAULT_MAX_VOLUME: u16 = 100;
const MAX_ALLOWED_VOLUME: u16 = 200;
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(test)]
const PADDING: (f64, f64) = (8.0, 8.0);
const DEFAULT_ICON_GAP_CHARS: usize = 1;
const PERCENT_TOKEN_CHARS: usize = 4;
const FILLED_SLIDER_COLOR: Color = Color::rgb(0.98, 0.82, 0.53);
const EMPTY_SLIDER_COLOR: Color = Color::rgb(0.55, 0.58, 0.67);
const MUTED_COLOR: Color = Color::rgb(0.93, 0.42, 0.42);
const UNAVAILABLE_COLOR: Color = Color::rgb(0.82, 0.43, 0.43);

#[derive(Debug, Clone)]
pub struct VolumeStyle {
    pub filled_color: Color,
    pub empty_color: Color,
    pub muted_color: Color,
    pub unavailable_color: Color,
}

impl VolumeStyle {
    pub fn from_config(config: &ModuleConfig) -> Self {
        Self {
            filled_color: resolve_color(
                config.filled_color.as_deref(),
                FILLED_SLIDER_COLOR,
                "module.volume.filled_color",
            ),
            empty_color: resolve_color(
                config.empty_color.as_deref(),
                EMPTY_SLIDER_COLOR,
                "module.volume.empty_color",
            ),
            muted_color: resolve_color(
                config.muted_color.as_deref(),
                MUTED_COLOR,
                "module.volume.muted_color",
            ),
            unavailable_color: resolve_color(
                config.unavailable_color.as_deref(),
                UNAVAILABLE_COLOR,
                "module.volume.unavailable_color",
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VolumeIcons {
    pub muted: String,
    pub unknown: String,
    pub low: String,
    pub medium: String,
    pub high: String,
    pub unavailable: String,
}

impl Default for VolumeIcons {
    fn default() -> Self {
        Self {
            muted: "󰝟".to_string(),
            unknown: "".to_string(),
            low: "󰕿".to_string(),
            medium: "󰖀".to_string(),
            high: "󰕾".to_string(),
            unavailable: "".to_string(),
        }
    }
}

impl VolumeIcons {
    pub fn from_config(config: &ModuleConfig) -> Self {
        let mut icons = Self::default();

        if let Some(value) = &config.icon_muted {
            icons.muted = value.clone();
        }
        if let Some(value) = &config.icon_unknown {
            icons.unknown = value.clone();
        }
        if let Some(value) = &config.icon_low {
            icons.low = value.clone();
        }
        if let Some(value) = &config.icon_medium {
            icons.medium = value.clone();
        }
        if let Some(value) = &config.icon_high {
            icons.high = value.clone();
        }
        if let Some(value) = &config.icon_unavailable {
            icons.unavailable = value.clone();
        }

        icons
    }

    fn slot_width(&self) -> usize {
        [
            char_len(&self.muted),
            char_len(&self.unknown),
            char_len(&self.low),
            char_len(&self.medium),
            char_len(&self.high),
            char_len(&self.unavailable),
        ]
        .into_iter()
        .max()
        .unwrap_or(0)
        .max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VolumeBackend {
    Wpctl,
    Pactl,
    Unavailable,
}

impl VolumeBackend {
    fn from_config(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "wpctl" => Some(Self::Wpctl),
            "pactl" => Some(Self::Pactl),
            "auto" => None,
            _ => None,
        }
    }

    fn binary(self) -> Option<&'static str> {
        match self {
            Self::Wpctl => Some("wpctl"),
            Self::Pactl => Some("pactl"),
            Self::Unavailable => None,
        }
    }

    fn default_device(self, target: Target) -> &'static str {
        match (self, target) {
            (Self::Wpctl, Target::Sink) => DEFAULT_DEVICE_WPCTL,
            (Self::Pactl, Target::Sink) => DEFAULT_DEVICE_PACTL,
            (Self::Unavailable, Target::Sink) => DEFAULT_DEVICE_WPCTL,
            (Self::Wpctl, Target::Source) => DEFAULT_SOURCE_WPCTL,
            (Self::Pactl, Target::Source) => DEFAULT_SOURCE_PACTL,
            (Self::Unavailable, Target::Source) => DEFAULT_SOURCE_WPCTL,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Wpctl => "wpctl",
            Self::Pactl => "pactl",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VolumeState {
    volume_percent: u16,
    muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickAction {
    ToggleMute,
    SetVolume(u16),
}

const DEFAULT_SCROLL_STEP: u16 = 5;

pub struct VolumeModule {
    backend: VolumeBackend,
    target: Target,
    device: String,
    slider_width: usize,
    show_slider: bool,
    scroll_step: u16,
    max_volume: u16,
    volume_percent: u16,
    muted: bool,
    state_known: bool,
    last_refresh: Option<Instant>,
    logged_refresh_error: bool,
    chrome: ModuleChrome,
    icons: VolumeIcons,
    icon_gap_chars: usize,
    glyphs: SliderGlyphs,
    style: VolumeStyle,
}

impl VolumeModule {
    pub fn new(
        backend: Option<String>,
        device: Option<String>,
        slider_width: Option<u32>,
        max_volume: Option<u16>,
        chrome: ModuleChrome,
        icons: VolumeIcons,
        glyphs: SliderGlyphs,
        config: &ModuleConfig,
    ) -> Self {
        let backend = detect_backend(backend.as_deref());
        let target = Target::from_config(config.target.as_deref());
        let slider_width = slider_width
            .unwrap_or(DEFAULT_SLIDER_WIDTH as u32)
            .clamp(1, 64) as usize;
        let max_volume = max_volume
            .unwrap_or(DEFAULT_MAX_VOLUME)
            .clamp(1, MAX_ALLOWED_VOLUME);
        let device = device.unwrap_or_else(|| backend.default_device(target).to_string());
        let show_slider = config.show_slider.unwrap_or(true);
        let scroll_step = config
            .scroll_step
            .unwrap_or(DEFAULT_SCROLL_STEP)
            .clamp(1, 100);

        Self {
            backend,
            target,
            device,
            slider_width,
            show_slider,
            scroll_step,
            max_volume,
            volume_percent: 0,
            muted: false,
            state_known: false,
            last_refresh: None,
            logged_refresh_error: false,
            chrome,
            icons,
            icon_gap_chars: config.icon_gap.unwrap_or(DEFAULT_ICON_GAP_CHARS as u32) as usize,
            glyphs,
            style: VolumeStyle::from_config(config),
        }
    }

    /// Compute the new percent after a wheel tick. `delta > 0` raises volume.
    fn nudged_percent(&self, delta: i32) -> u16 {
        let cur = self.volume_percent as i32;
        let stepped = cur + delta * self.scroll_step as i32;
        stepped.clamp(0, self.max_volume as i32) as u16
    }

    fn icon_text(&self) -> &str {
        if self.muted {
            &self.icons.muted
        } else if !self.state_known {
            &self.icons.unknown
        } else {
            let ratio = self.volume_percent.min(self.max_volume) as f64 / self.max_volume as f64;
            if ratio < 0.34 {
                &self.icons.low
            } else if ratio < 0.67 {
                &self.icons.medium
            } else {
                &self.icons.high
            }
        }
    }

    fn icon_slot_chars(&self) -> usize {
        self.icons.slot_width()
    }

    fn icon_slot(&self) -> String {
        pad_text_right(self.icon_text(), self.icon_slot_chars())
    }

    fn icon_gap(&self) -> String {
        " ".repeat(self.icon_gap_chars)
    }

    fn percent_text(&self) -> String {
        if self.state_known {
            format!("{:>3}%", self.volume_percent.min(999))
        } else {
            " --%".to_string()
        }
    }

    fn slider_fill(&self) -> usize {
        if self.max_volume == 0 {
            return 0;
        }

        let ratio = self.volume_percent.min(self.max_volume) as f64 / self.max_volume as f64;
        (ratio * self.slider_width as f64)
            .round()
            .clamp(0.0, self.slider_width as f64) as usize
    }

    fn total_content_chars(&self) -> usize {
        self.icon_slot_chars()
            + self.icon_gap_chars
            + self.glyphs.total_chars(self.slider_width)
            + 1
            + PERCENT_TOKEN_CHARS
    }

    fn refresh_state(&mut self) {
        self.last_refresh = Some(Instant::now());

        let result = match self.backend {
            VolumeBackend::Wpctl => read_wpctl_state(&self.device),
            VolumeBackend::Pactl => read_pactl_state(&self.device, self.target),
            VolumeBackend::Unavailable => {
                Err("No supported audio backend found in PATH; install wpctl or pactl".to_string())
            }
        };

        match result {
            Ok(state) => {
                self.volume_percent = state.volume_percent;
                self.muted = state.muted;
                self.state_known = true;
                self.logged_refresh_error = false;
            }
            Err(error) => {
                if self.backend != VolumeBackend::Unavailable && !self.logged_refresh_error {
                    log::warn!(
                        "Failed to refresh volume state via {} (device '{}'): {}",
                        self.backend.display_name(),
                        self.device,
                        error
                    );
                    self.logged_refresh_error = true;
                }
            }
        }
    }

    fn maybe_refresh(&mut self) {
        let should_refresh = self
            .last_refresh
            .map(|last| last.elapsed() >= REFRESH_INTERVAL)
            .unwrap_or(true);
        if should_refresh {
            self.refresh_state();
        }
    }

    fn set_volume_percent(&mut self, percent: u16) {
        if self.backend == VolumeBackend::Unavailable {
            return;
        }

        let target_pct = percent.min(self.max_volume);
        let (pa_set_vol, pa_set_mute) = match self.target {
            Target::Sink => ("set-sink-volume", "set-sink-mute"),
            Target::Source => ("set-source-volume", "set-source-mute"),
        };
        let result = match self.backend {
            VolumeBackend::Wpctl => {
                let level = format!("{:.2}", target_pct as f64 / 100.0);
                run_command("wpctl", &["set-volume", &self.device, &level])
                    .and_then(|_| run_command("wpctl", &["set-mute", &self.device, "0"]))
            }
            VolumeBackend::Pactl => {
                let level = format!("{target_pct}%");
                run_command("pactl", &[pa_set_vol, &self.device, &level])
                    .and_then(|_| run_command("pactl", &[pa_set_mute, &self.device, "no"]))
            }
            VolumeBackend::Unavailable => Ok(String::new()),
        };

        match result {
            Ok(_) => self.refresh_state(),
            Err(error) => {
                self.last_refresh = Some(Instant::now());
                log::warn!(
                    "Failed to set volume via {} (device '{}'): {}",
                    self.backend.display_name(),
                    self.device,
                    error
                );
            }
        }
    }

    fn toggle_mute(&mut self) {
        if self.backend == VolumeBackend::Unavailable {
            return;
        }

        let pa_set_mute = match self.target {
            Target::Sink => "set-sink-mute",
            Target::Source => "set-source-mute",
        };
        let result = match self.backend {
            VolumeBackend::Wpctl => run_command("wpctl", &["set-mute", &self.device, "toggle"]),
            VolumeBackend::Pactl => run_command("pactl", &[pa_set_mute, &self.device, "toggle"]),
            VolumeBackend::Unavailable => Ok(String::new()),
        };

        match result {
            Ok(_) => self.refresh_state(),
            Err(error) => {
                self.last_refresh = Some(Instant::now());
                log::warn!(
                    "Failed to toggle mute via {} (device '{}'): {}",
                    self.backend.display_name(),
                    self.device,
                    error
                );
            }
        }
    }

    fn action_for_click(&self, event: &ClickEvent) -> Option<ClickAction> {
        if self.backend == VolumeBackend::Unavailable {
            return None;
        }

        let content_width =
            (event.module_width - self.chrome.padding.0 - self.chrome.padding.1).max(0.0);
        if content_width <= 0.0 {
            return None;
        }

        let rel_x = event.x - self.chrome.padding.0;
        if !(0.0..content_width).contains(&rel_x) {
            return None;
        }

        // Compact form (no slider): whole module is a mute toggle.
        if !self.show_slider {
            return Some(ClickAction::ToggleMute);
        }

        let char_width = content_width / self.total_content_chars() as f64;
        if char_width <= 0.0 {
            return None;
        }

        let icon_width = (self.icon_slot_chars() + self.icon_gap_chars) as f64 * char_width;
        if rel_x < icon_width {
            return Some(ClickAction::ToggleMute);
        }

        let slider_start = (self.icon_slot_chars() + self.icon_gap_chars) as f64 * char_width;
        let slider_end =
            slider_start + self.glyphs.total_chars(self.slider_width) as f64 * char_width;
        if rel_x < slider_start || rel_x >= slider_end {
            return None;
        }

        let inner_start = slider_start + char_len(&self.glyphs.left) as f64 * char_width;
        let inner_end =
            inner_start + (self.slider_width * self.glyphs.unit_chars()) as f64 * char_width;
        let ratio = if rel_x <= inner_start {
            0.0
        } else if rel_x >= inner_end {
            1.0
        } else {
            (rel_x - inner_start) / (inner_end - inner_start)
        };

        Some(ClickAction::SetVolume(
            (ratio * self.max_volume as f64).round() as u16,
        ))
    }
}

impl Module for VolumeModule {
    fn update(&mut self) {
        self.maybe_refresh();
    }

    fn update_interval(&self) -> std::time::Duration {
        self.chrome
            .update_interval
            .unwrap_or(std::time::Duration::from_secs(1))
    }

    fn view(&self) -> ModuleView {
        if self.backend == VolumeBackend::Unavailable {
            return self.chrome.apply(ModuleView {
                text: prefix_text(&self.icons.unavailable, "unavailable"),
                style: TextStyle {
                    color: self.style.unavailable_color,
                    ..TextStyle::default()
                },
                ..Default::default()
            });
        }

        let base_style = TextStyle {
            color: self.chrome.foreground.unwrap_or(Color::WHITE),
            ..TextStyle::default()
        };
        let muted_style = TextStyle {
            color: self.style.muted_color,
            ..base_style.clone()
        };
        let label_style = if self.muted { muted_style.clone() } else { base_style.clone() };
        let percent_style = if self.muted { muted_style.clone() } else { base_style.clone() };

        let icon_slot = self.icon_slot();
        let icon_gap = self.icon_gap();
        let percent_text = self.percent_text();

        if !self.show_slider {
            // Compact form: `<icon> <pct>%` — matches waybar's pulseaudio
            // visuals (no progress bar).
            let full_text = format!("{icon_slot}{icon_gap}{percent_text}");
            return self.chrome.apply(ModuleView {
                text: full_text,
                text_segments: vec![
                    TextSegment {
                        text: icon_slot,
                        style: label_style,
                    },
                    TextSegment {
                        text: icon_gap,
                        style: base_style.clone(),
                    },
                    TextSegment {
                        text: percent_text,
                        style: percent_style,
                    },
                ],
                style: base_style,
                ..Default::default()
            });
        }

        let filled_style = if self.muted {
            muted_style.clone()
        } else {
            TextStyle {
                color: self.style.filled_color,
                ..base_style.clone()
            }
        };
        let empty_style = TextStyle {
            color: self.style.empty_color,
            ..base_style.clone()
        };

        let filled = self.slider_fill();
        let empty = self.slider_width.saturating_sub(filled);
        let left = self.glyphs.left.clone();
        let filled_text = self.glyphs.filled.repeat(filled);
        let empty_text = self.glyphs.empty.repeat(empty);
        let right = format!("{} ", self.glyphs.right);
        let full_text =
            format!("{icon_slot}{icon_gap}{left}{filled_text}{empty_text}{right}{percent_text}");

        self.chrome.apply(ModuleView {
            text: full_text,
            text_segments: vec![
                TextSegment {
                    text: icon_slot,
                    style: label_style,
                },
                TextSegment {
                    text: icon_gap,
                    style: base_style.clone(),
                },
                TextSegment {
                    text: left,
                    style: base_style.clone(),
                },
                TextSegment {
                    text: filled_text,
                    style: filled_style,
                },
                TextSegment {
                    text: empty_text,
                    style: empty_style,
                },
                TextSegment {
                    text: right,
                    style: base_style.clone(),
                },
                TextSegment {
                    text: percent_text,
                    style: percent_style,
                },
            ],
            style: base_style,
            ..Default::default()
        })
    }

    fn click(&mut self, event: ClickEvent) {
        match event.button {
            MouseButton::Left => match self.action_for_click(&event) {
                Some(ClickAction::ToggleMute) => self.toggle_mute(),
                Some(ClickAction::SetVolume(percent)) => self.set_volume_percent(percent),
                None => {}
            },
            // Wheel up/down anywhere on the module nudges the volume — matches
            // waybar's `on-scroll-up` / `on-scroll-down` behaviour.
            MouseButton::ScrollUp => {
                if self.state_known {
                    self.set_volume_percent(self.nudged_percent(1));
                }
            }
            MouseButton::ScrollDown => {
                if self.state_known {
                    self.set_volume_percent(self.nudged_percent(-1));
                }
            }
            _ => {}
        }
    }
}

fn detect_backend(preferred: Option<&str>) -> VolumeBackend {
    if let Some(value) = preferred {
        if let Some(backend) = VolumeBackend::from_config(value) {
            if let Some(binary) = backend.binary() {
                if command_exists(binary) {
                    return backend;
                }
                log::warn!(
                    "Configured volume backend '{}' is not installed; falling back to auto-detect",
                    value
                );
            }
        } else {
            log::warn!(
                "Unknown volume backend '{}' configured; falling back to auto-detect",
                value
            );
        }
    }

    if command_exists("wpctl") {
        VolumeBackend::Wpctl
    } else if command_exists("pactl") {
        VolumeBackend::Pactl
    } else {
        VolumeBackend::Unavailable
    }
}

fn command_exists(command: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|path| path.join(command).is_file()))
        .unwrap_or(false)
}

fn run_command(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("{program} exited with status {}", output.status))
        } else {
            Err(stderr)
        }
    }
}

fn read_wpctl_state(device: &str) -> Result<VolumeState, String> {
    let output = run_command("wpctl", &["get-volume", device])?;
    parse_wpctl_output(&output).ok_or_else(|| format!("Unexpected wpctl output: {output}"))
}

fn read_pactl_state(device: &str, target: Target) -> Result<VolumeState, String> {
    let (volume_cmd, mute_cmd) = match target {
        Target::Sink => ("get-sink-volume", "get-sink-mute"),
        Target::Source => ("get-source-volume", "get-source-mute"),
    };
    let volume_output = run_command("pactl", &[volume_cmd, device])?;
    let mute_output = run_command("pactl", &[mute_cmd, device])?;
    let volume_percent = parse_pactl_volume_output(&volume_output)
        .ok_or_else(|| format!("Unexpected pactl volume output: {volume_output}"))?;
    let muted = parse_pactl_mute_output(&mute_output)
        .ok_or_else(|| format!("Unexpected pactl mute output: {mute_output}"))?;

    Ok(VolumeState {
        volume_percent,
        muted,
    })
}

fn parse_wpctl_output(output: &str) -> Option<VolumeState> {
    let mut parts = output.split_whitespace();
    if parts.next()? != "Volume:" {
        return None;
    }

    let raw = parts.next()?.parse::<f64>().ok()?;
    let muted = parts.any(|part| part.contains("MUTED"));

    Some(VolumeState {
        volume_percent: (raw * 100.0).round().max(0.0) as u16,
        muted,
    })
}

fn parse_pactl_volume_output(output: &str) -> Option<u16> {
    output
        .split('/')
        .nth(1)?
        .trim()
        .trim_end_matches('%')
        .parse::<u16>()
        .ok()
}

fn parse_pactl_mute_output(output: &str) -> Option<bool> {
    output
        .trim()
        .strip_prefix("Mute:")?
        .trim()
        .parse::<MuteFlag>()
        .ok()
        .map(|flag| flag.0)
}

struct MuteFlag(bool);

impl std::str::FromStr for MuteFlag {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "yes" => Ok(Self(true)),
            "no" => Ok(Self(false)),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ModuleConfig;
    use crate::core::module::{ModuleChrome, SliderGlyphs};

    fn default_module_config() -> ModuleConfig {
        ModuleConfig {
            kind: "volume".to_string(),
            ..Default::default()
        }
    }

    fn default_chrome() -> ModuleChrome {
        ModuleChrome {
            foreground: None,
            background: None,
            padding: PADDING,
            icon_spacing: None,
            update_interval: None,
        }
    }

    fn click_event(x: f64, module_width: f64) -> ClickEvent {
        ClickEvent {
            x,
            bar_x: x,
            screen_x: x,
            module_width,
            y: 0.0,
            bar_y: 0.0,
            screen_y: 0.0,
            button: MouseButton::Left,
        }
    }

    fn module_width(module: &VolumeModule) -> f64 {
        module.chrome.padding.0 + module.chrome.padding.1 + module.total_content_chars() as f64
    }

    fn slider_click_x(module: &VolumeModule, ratio: f64) -> f64 {
        let inner_start = module.chrome.padding.0
            + module.icon_slot_chars() as f64
            + module.icon_gap_chars as f64
            + char_len(&module.glyphs.left) as f64;
        inner_start
            + ratio.clamp(0.0, 1.0) * (module.slider_width * module.glyphs.unit_chars()) as f64
    }

    #[test]
    fn parses_wpctl_volume() {
        assert_eq!(
            parse_wpctl_output("Volume: 1.00"),
            Some(VolumeState {
                volume_percent: 100,
                muted: false,
            })
        );
        assert_eq!(
            parse_wpctl_output("Volume: 0.42 [MUTED]"),
            Some(VolumeState {
                volume_percent: 42,
                muted: true,
            })
        );
    }

    #[test]
    fn parses_pactl_volume_and_mute() {
        let volume =
            "Volume: front-left: 65536 / 100% / 0.00 dB,   front-right: 65536 / 100% / 0.00 dB";
        assert_eq!(parse_pactl_volume_output(volume), Some(100));
        assert_eq!(parse_pactl_mute_output("Mute: yes"), Some(true));
        assert_eq!(parse_pactl_mute_output("Mute: no"), Some(false));
    }

    #[test]
    fn click_on_label_toggles_mute() {
        let module = VolumeModule::new(
            None,
            None,
            Some(20),
            Some(100),
            default_chrome(),
            VolumeIcons::default(),
            SliderGlyphs::new("▐", "█", "░", "▌"),
            &default_module_config(),
        );
        assert_eq!(
            module.action_for_click(&click_event(PADDING.0 + 0.5, module_width(&module))),
            Some(ClickAction::ToggleMute)
        );
    }

    #[test]
    fn click_on_slider_maps_to_volume() {
        let module = VolumeModule::new(
            None,
            None,
            Some(20),
            Some(100),
            default_chrome(),
            VolumeIcons::default(),
            SliderGlyphs::new("▐", "█", "░", "▌"),
            &default_module_config(),
        );
        let module_width = module_width(&module);

        assert_eq!(
            module.action_for_click(&click_event(slider_click_x(&module, 0.0), module_width)),
            Some(ClickAction::SetVolume(0))
        );
        assert_eq!(
            module.action_for_click(&click_event(slider_click_x(&module, 0.5), module_width)),
            Some(ClickAction::SetVolume(50))
        );
        assert_eq!(
            module.action_for_click(&click_event(slider_click_x(&module, 1.0), module_width)),
            Some(ClickAction::SetVolume(100))
        );
    }

    #[test]
    fn uses_configured_icons() {
        let mut icons = VolumeIcons::default();
        icons.high = "VOL".to_string();
        let mut module = VolumeModule::new(
            None,
            None,
            Some(20),
            Some(100),
            default_chrome(),
            icons,
            SliderGlyphs::new("▐", "█", "░", "▌"),
            &default_module_config(),
        );
        module.state_known = true;
        module.volume_percent = 100;

        let view = module.view();

        assert!(view.text.starts_with("VOL "));
    }

    #[test]
    fn uses_configured_slider_glyphs() {
        let mut module = VolumeModule::new(
            None,
            None,
            Some(3),
            Some(100),
            default_chrome(),
            VolumeIcons::default(),
            SliderGlyphs::new("[", "=", ".", "]"),
            &default_module_config(),
        );
        module.state_known = true;
        module.volume_percent = 50;

        let view = module.view();

        assert!(view.text.contains("["));
        assert!(view.text.contains("]"));
        assert!(view.text.contains("="));
    }
}
