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

const DEFAULT_CLASS: &str = "backlight";
const DEFAULT_SLIDER_WIDTH: usize = 20;
const DEFAULT_MAX_BRIGHTNESS: u16 = 100;
const MAX_ALLOWED_BRIGHTNESS: u16 = 100;
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(test)]
const PADDING: (f64, f64) = (8.0, 8.0);
const DEFAULT_ICON_GAP_CHARS: usize = 1;
const PERCENT_TOKEN_CHARS: usize = 4;
const FILLED_SLIDER_COLOR: Color = Color::rgb(0.99, 0.86, 0.45);
const EMPTY_SLIDER_COLOR: Color = Color::rgb(0.55, 0.58, 0.67);
const UNAVAILABLE_COLOR: Color = Color::rgb(0.82, 0.43, 0.43);

#[derive(Debug, Clone)]
pub struct BrightnessStyle {
    pub filled_color: Color,
    pub empty_color: Color,
    pub unavailable_color: Color,
}

impl BrightnessStyle {
    pub fn from_config(config: &ModuleConfig) -> Self {
        Self {
            filled_color: resolve_color(
                config.filled_color.as_deref(),
                FILLED_SLIDER_COLOR,
                "module.brightness.filled_color",
            ),
            empty_color: resolve_color(
                config.empty_color.as_deref(),
                EMPTY_SLIDER_COLOR,
                "module.brightness.empty_color",
            ),
            unavailable_color: resolve_color(
                config.unavailable_color.as_deref(),
                UNAVAILABLE_COLOR,
                "module.brightness.unavailable_color",
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BrightnessIcons {
    pub low: String,
    pub medium: String,
    pub high: String,
    pub unknown: String,
    pub unavailable: String,
}

impl Default for BrightnessIcons {
    fn default() -> Self {
        Self {
            low: "󰃚".to_string(),
            medium: "󰃝".to_string(),
            high: "󰃠".to_string(),
            unknown: "".to_string(),
            unavailable: "".to_string(),
        }
    }
}

impl BrightnessIcons {
    pub fn from_config(config: &ModuleConfig) -> Self {
        let mut icons = Self::default();
        let base = config.icon.clone();

        if let Some(value) = &config.icon {
            icons.low = value.clone();
            icons.medium = value.clone();
            icons.high = value.clone();
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
        if let Some(value) = &base {
            if config.icon_low.is_none() {
                icons.low = value.clone();
            }
            if config.icon_medium.is_none() {
                icons.medium = value.clone();
            }
            if config.icon_high.is_none() {
                icons.high = value.clone();
            }
        }
        if let Some(value) = &config.icon_unknown {
            icons.unknown = value.clone();
        }
        if let Some(value) = &config.icon_unavailable {
            icons.unavailable = value.clone();
        }

        icons
    }

    fn slot_width(&self) -> usize {
        [
            char_len(&self.low),
            char_len(&self.medium),
            char_len(&self.high),
            char_len(&self.unknown),
            char_len(&self.unavailable),
        ]
        .into_iter()
        .max()
        .unwrap_or(0)
        .max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrightnessBackend {
    /// Read/write directly via /sys/class/backlight/<device>/. Works on most
    /// Linux systems out of the box and doesn't need an external binary.
    Sysfs,
    Brightnessctl,
    Unavailable,
}

impl BrightnessBackend {
    fn from_config(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sysfs" => Some(Self::Sysfs),
            "brightnessctl" => Some(Self::Brightnessctl),
            "auto" => None,
            _ => None,
        }
    }

    fn binary(self) -> Option<&'static str> {
        match self {
            Self::Brightnessctl => Some("brightnessctl"),
            Self::Sysfs | Self::Unavailable => None,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Sysfs => "sysfs",
            Self::Brightnessctl => "brightnessctl",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BrightnessState {
    brightness_percent: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickAction {
    SetBrightness(u16),
}

pub struct BrightnessModule {
    backend: BrightnessBackend,
    device: Option<String>,
    slider_width: usize,
    max_brightness: u16,
    brightness_percent: u16,
    state_known: bool,
    last_refresh: Option<Instant>,
    logged_refresh_error: bool,
    chrome: ModuleChrome,
    icons: BrightnessIcons,
    icon_gap_chars: usize,
    glyphs: SliderGlyphs,
    style: BrightnessStyle,
}

impl BrightnessModule {
    pub fn new(
        backend: Option<String>,
        device: Option<String>,
        slider_width: Option<u32>,
        max_brightness: Option<u16>,
        chrome: ModuleChrome,
        icons: BrightnessIcons,
        glyphs: SliderGlyphs,
        config: &ModuleConfig,
    ) -> Self {
        let backend = detect_backend(backend.as_deref());
        let slider_width = slider_width
            .unwrap_or(DEFAULT_SLIDER_WIDTH as u32)
            .clamp(1, 64) as usize;
        let max_brightness = max_brightness
            .unwrap_or(DEFAULT_MAX_BRIGHTNESS)
            .clamp(1, MAX_ALLOWED_BRIGHTNESS);

        Self {
            backend,
            device,
            slider_width,
            max_brightness,
            brightness_percent: 0,
            state_known: false,
            last_refresh: None,
            logged_refresh_error: false,
            chrome,
            icons,
            icon_gap_chars: config.icon_gap.unwrap_or(DEFAULT_ICON_GAP_CHARS as u32) as usize,
            glyphs,
            style: BrightnessStyle::from_config(config),
        }
    }

    fn icon_text(&self) -> &str {
        if !self.state_known {
            &self.icons.unknown
        } else {
            let ratio = self.brightness_percent.min(self.max_brightness) as f64
                / self.max_brightness as f64;
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
            format!("{:>3}%", self.brightness_percent.min(999))
        } else {
            " --%".to_string()
        }
    }

    fn slider_fill(&self) -> usize {
        if self.max_brightness == 0 {
            return 0;
        }

        let ratio =
            self.brightness_percent.min(self.max_brightness) as f64 / self.max_brightness as f64;
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

    fn target_label(&self) -> String {
        self.device
            .as_deref()
            .map(|device| format!("device '{device}'"))
            .unwrap_or_else(|| format!("class '{DEFAULT_CLASS}'"))
    }

    fn refresh_state(&mut self) {
        self.last_refresh = Some(Instant::now());

        let result = match self.backend {
            BrightnessBackend::Sysfs => read_sysfs_state(self.device.as_deref()),
            BrightnessBackend::Brightnessctl => read_brightnessctl_state(self.device.as_deref()),
            BrightnessBackend::Unavailable => Err(
                "No supported brightness backend found (need /sys/class/backlight or brightnessctl)"
                    .to_string(),
            ),
        };

        match result {
            Ok(state) => {
                self.brightness_percent = state.brightness_percent;
                self.state_known = true;
                self.logged_refresh_error = false;
            }
            Err(error) => {
                if self.backend != BrightnessBackend::Unavailable && !self.logged_refresh_error {
                    log::warn!(
                        "Failed to refresh brightness state via {} ({}): {}",
                        self.backend.display_name(),
                        self.target_label(),
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

    fn set_brightness_percent(&mut self, percent: u16) {
        if self.backend == BrightnessBackend::Unavailable {
            return;
        }

        let target = percent.min(self.max_brightness);
        let result = match self.backend {
            BrightnessBackend::Sysfs => {
                write_sysfs_brightness(self.device.as_deref(), target).map(|_| String::new())
            }
            BrightnessBackend::Brightnessctl => {
                let mut args = vec!["-q".to_string()];
                args.extend(target_args(self.device.as_deref()));
                args.push("set".to_string());
                args.push(format!("{target}%"));
                run_command_owned("brightnessctl", &args)
            }
            BrightnessBackend::Unavailable => Ok(String::new()),
        };

        match result {
            Ok(_) => self.refresh_state(),
            Err(error) => {
                self.last_refresh = Some(Instant::now());
                log::warn!(
                    "Failed to set brightness via {} ({}): {}",
                    self.backend.display_name(),
                    self.target_label(),
                    error
                );
            }
        }
    }

    fn action_for_click(&self, event: &ClickEvent) -> Option<ClickAction> {
        if self.backend == BrightnessBackend::Unavailable {
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

        let char_width = content_width / self.total_content_chars() as f64;
        if char_width <= 0.0 {
            return None;
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

        Some(ClickAction::SetBrightness(
            (ratio * self.max_brightness as f64).round() as u16,
        ))
    }
}

impl Module for BrightnessModule {
    fn update(&mut self) {
        self.maybe_refresh();
    }

    fn view(&self) -> ModuleView {
        if self.backend == BrightnessBackend::Unavailable {
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
        let filled_style = TextStyle {
            color: self.style.filled_color,
            ..base_style.clone()
        };
        let empty_style = TextStyle {
            color: self.style.empty_color,
            ..base_style.clone()
        };

        let filled = self.slider_fill();
        let empty = self.slider_width.saturating_sub(filled);
        let icon_slot = self.icon_slot();
        let icon_gap = self.icon_gap();
        let percent_text = self.percent_text();
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
                    style: base_style.clone(),
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
                    style: base_style.clone(),
                },
            ],
            style: base_style,
            ..Default::default()
        })
    }

    fn click(&mut self, event: ClickEvent) {
        if event.button != MouseButton::Left {
            return;
        }

        match self.action_for_click(&event) {
            Some(ClickAction::SetBrightness(percent)) => self.set_brightness_percent(percent),
            None => {}
        }
    }
}

fn detect_backend(preferred: Option<&str>) -> BrightnessBackend {
    if let Some(value) = preferred {
        if let Some(backend) = BrightnessBackend::from_config(value) {
            match backend {
                BrightnessBackend::Sysfs => {
                    if sysfs_backlight_root().exists() {
                        return BrightnessBackend::Sysfs;
                    }
                    log::warn!(
                        "Configured brightness backend 'sysfs' but /sys/class/backlight is missing; falling back to auto-detect",
                    );
                }
                BrightnessBackend::Brightnessctl => {
                    if command_exists("brightnessctl") {
                        return BrightnessBackend::Brightnessctl;
                    }
                    log::warn!(
                        "Configured brightness backend 'brightnessctl' is not installed; falling back to auto-detect",
                    );
                }
                BrightnessBackend::Unavailable => {}
            }
        } else {
            log::warn!(
                "Unknown brightness backend '{}' configured; falling back to auto-detect",
                value
            );
        }
    }

    // Prefer sysfs: no external dependency, works on every distro.
    if sysfs_has_backlight() {
        BrightnessBackend::Sysfs
    } else if command_exists("brightnessctl") {
        BrightnessBackend::Brightnessctl
    } else {
        BrightnessBackend::Unavailable
    }
}

fn sysfs_backlight_root() -> std::path::PathBuf {
    std::path::PathBuf::from("/sys/class/backlight")
}

fn sysfs_has_backlight() -> bool {
    std::fs::read_dir(sysfs_backlight_root())
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

/// Resolve which sysfs backlight device to use. Returns `None` if no backlight
/// devices are present at all.
fn resolve_sysfs_device(preferred: Option<&str>) -> Option<std::path::PathBuf> {
    let root = sysfs_backlight_root();
    if let Some(name) = preferred {
        let path = root.join(name);
        if path.is_dir() {
            return Some(path);
        }
    }
    let mut entries: Vec<_> = std::fs::read_dir(&root).ok()?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    let first = entries.into_iter().next()?;
    Some(first.path())
}

fn read_sysfs_state(device: Option<&str>) -> Result<BrightnessState, String> {
    let dir = resolve_sysfs_device(device)
        .ok_or_else(|| "No /sys/class/backlight devices present".to_string())?;
    let max_raw = std::fs::read_to_string(dir.join("max_brightness"))
        .map_err(|e| format!("failed to read max_brightness: {e}"))?;
    let cur_raw = std::fs::read_to_string(dir.join("brightness"))
        .map_err(|e| format!("failed to read brightness: {e}"))?;
    let max: u64 = max_raw
        .trim()
        .parse()
        .map_err(|e| format!("invalid max_brightness '{}': {e}", max_raw.trim()))?;
    let cur: u64 = cur_raw
        .trim()
        .parse()
        .map_err(|e| format!("invalid brightness '{}': {e}", cur_raw.trim()))?;
    if max == 0 {
        return Err("max_brightness is 0".to_string());
    }
    let percent = ((cur as f64 / max as f64) * 100.0).round().clamp(0.0, 100.0) as u16;
    Ok(BrightnessState {
        brightness_percent: percent,
    })
}

fn write_sysfs_brightness(device: Option<&str>, percent: u16) -> Result<(), String> {
    let dir = resolve_sysfs_device(device)
        .ok_or_else(|| "No /sys/class/backlight devices present".to_string())?;
    let max_raw = std::fs::read_to_string(dir.join("max_brightness"))
        .map_err(|e| format!("failed to read max_brightness: {e}"))?;
    let max: u64 = max_raw
        .trim()
        .parse()
        .map_err(|e| format!("invalid max_brightness '{}': {e}", max_raw.trim()))?;
    let value = ((percent.min(100) as u64 * max) as f64 / 100.0).round() as u64;
    std::fs::write(dir.join("brightness"), value.to_string()).map_err(|e| {
        format!(
            "failed to write brightness (need write access to {}): {e}",
            dir.join("brightness").display()
        )
    })
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

fn run_command_owned(program: &str, args: &[String]) -> Result<String, String> {
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_command(program, &arg_refs)
}

fn target_args(device: Option<&str>) -> Vec<String> {
    match device {
        Some(device) => vec!["-d".to_string(), device.to_string()],
        None => vec!["-c".to_string(), DEFAULT_CLASS.to_string()],
    }
}

fn read_brightnessctl_state(device: Option<&str>) -> Result<BrightnessState, String> {
    let mut args = target_args(device);
    args.push("-m".to_string());
    args.push("info".to_string());

    let output = run_command_owned("brightnessctl", &args)?;
    parse_brightnessctl_output(&output)
        .ok_or_else(|| format!("Unexpected brightnessctl output: {output}"))
}

fn parse_brightnessctl_output(output: &str) -> Option<BrightnessState> {
    let line = output.lines().find(|line| !line.trim().is_empty())?;
    let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
    if fields.len() < 5 {
        return None;
    }

    let brightness_percent = fields[2]
        .parse::<f64>()
        .ok()
        .zip(fields[4].parse::<f64>().ok())
        .and_then(|(current, max)| {
            if max <= 0.0 {
                None
            } else {
                Some(((current / max) * 100.0).round())
            }
        })
        .map(|percent| percent.clamp(0.0, 100.0) as u16)
        .or_else(|| {
            fields[3]
                .trim_end_matches('%')
                .parse::<u16>()
                .ok()
                .map(|percent| percent.min(100))
        })?;

    Some(BrightnessState { brightness_percent })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ModuleConfig;
    use crate::core::module::{ModuleChrome, SliderGlyphs};

    fn default_module_config() -> ModuleConfig {
        ModuleConfig {
            kind: "brightness".to_string(),
            ..Default::default()
        }
    }

    fn default_chrome() -> ModuleChrome {
        ModuleChrome {
            foreground: None,
            background: None,
            padding: PADDING,
            icon_spacing: None,
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

    fn module_width(module: &BrightnessModule) -> f64 {
        module.chrome.padding.0 + module.chrome.padding.1 + module.total_content_chars() as f64
    }

    fn slider_click_x(module: &BrightnessModule, ratio: f64) -> f64 {
        let inner_start = module.chrome.padding.0
            + module.icon_slot_chars() as f64
            + module.icon_gap_chars as f64
            + char_len(&module.glyphs.left) as f64;
        inner_start
            + ratio.clamp(0.0, 1.0) * (module.slider_width * module.glyphs.unit_chars()) as f64
    }

    #[test]
    fn parses_brightnessctl_output() {
        assert_eq!(
            parse_brightnessctl_output("intel_backlight,backlight,937,43%,2200"),
            Some(BrightnessState {
                brightness_percent: 43,
            })
        );
    }

    #[test]
    fn click_on_label_does_nothing() {
        let module = BrightnessModule::new(
            None,
            None,
            Some(20),
            Some(100),
            default_chrome(),
            BrightnessIcons::default(),
            SliderGlyphs::new("▐", "█", "░", "▌"),
            &default_module_config(),
        );
        assert_eq!(
            module.action_for_click(&click_event(PADDING.0 + 0.5, module_width(&module))),
            None
        );
    }

    #[test]
    fn click_on_slider_maps_to_brightness() {
        let module = BrightnessModule::new(
            None,
            None,
            Some(20),
            Some(100),
            default_chrome(),
            BrightnessIcons::default(),
            SliderGlyphs::new("▐", "█", "░", "▌"),
            &default_module_config(),
        );
        let module_width = module_width(&module);

        assert_eq!(
            module.action_for_click(&click_event(slider_click_x(&module, 0.0), module_width)),
            Some(ClickAction::SetBrightness(0))
        );
        assert_eq!(
            module.action_for_click(&click_event(slider_click_x(&module, 0.5), module_width)),
            Some(ClickAction::SetBrightness(50))
        );
        assert_eq!(
            module.action_for_click(&click_event(slider_click_x(&module, 1.0), module_width)),
            Some(ClickAction::SetBrightness(100))
        );
    }

    #[test]
    fn uses_configured_icon() {
        let mut icons = BrightnessIcons::default();
        icons.high = "BR".to_string();
        let mut module = BrightnessModule::new(
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
        module.brightness_percent = 100;

        let view = module.view();

        assert!(view.text.starts_with("BR "));
    }

    #[test]
    fn uses_configured_slider_glyphs() {
        let mut module = BrightnessModule::new(
            None,
            None,
            Some(3),
            Some(100),
            default_chrome(),
            BrightnessIcons::default(),
            SliderGlyphs::new("[", "=", ".", "]"),
            &default_module_config(),
        );
        module.state_known = true;
        module.brightness_percent = 50;

        let view = module.view();

        assert!(view.text.contains("["));
        assert!(view.text.contains("]"));
        assert!(view.text.contains("="));
    }
}
