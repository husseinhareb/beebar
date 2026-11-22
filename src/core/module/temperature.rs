use sysinfo::Components;

use super::{Module, ModuleChrome, ModuleView, TextSegment, prefix_text};
use crate::core::config::{ModuleConfig, resolve_color};
use crate::renderer::color::Color;
use crate::renderer::primitives::TextStyle;

const DEFAULT_TEMP_ICON: &str = "";
const DEFAULT_FORMAT: &str = "{icon} {temp}°C";
const DEFAULT_WARN_THRESHOLD: f32 = 70.0;
const DEFAULT_CRITICAL_THRESHOLD: f32 = 90.0;
const WARN_COLOR: Color = Color::rgb(0.98, 0.82, 0.53);
const CRITICAL_COLOR: Color = Color::rgb(0.93, 0.42, 0.42);
const UNAVAILABLE_COLOR: Color = Color::rgb(0.82, 0.43, 0.43);

#[derive(Debug, Clone)]
pub struct TemperatureStyle {
    pub warn_color: Color,
    pub critical_color: Color,
    pub unavailable_color: Color,
}

impl TemperatureStyle {
    pub fn from_config(config: &ModuleConfig) -> Self {
        Self {
            warn_color: resolve_color(
                config.warn_color.as_deref(),
                WARN_COLOR,
                "module.temperature.warn_color",
            ),
            critical_color: resolve_color(
                config.critical_color.as_deref(),
                CRITICAL_COLOR,
                "module.temperature.critical_color",
            ),
            unavailable_color: resolve_color(
                config.unavailable_color.as_deref(),
                UNAVAILABLE_COLOR,
                "module.temperature.unavailable_color",
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TemperatureIcons {
    pub normal: String,
    pub warn: String,
    pub critical: String,
}

impl Default for TemperatureIcons {
    fn default() -> Self {
        Self {
            normal: DEFAULT_TEMP_ICON.to_string(),
            warn: "".to_string(),
            critical: "".to_string(),
        }
    }
}

impl TemperatureIcons {
    pub fn from_config(config: &ModuleConfig) -> Self {
        let mut icons = Self::default();

        // A single `icon` sets all three to the same glyph.
        if let Some(value) = &config.icon {
            icons.normal = value.clone();
            icons.warn = value.clone();
            icons.critical = value.clone();
        }
        if let Some(value) = &config.icon_low {
            icons.normal = value.clone();
        }
        if let Some(value) = &config.icon_medium {
            icons.warn = value.clone();
        }
        if let Some(value) = &config.icon_high {
            icons.critical = value.clone();
        }

        icons
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TemperatureState {
    current: f32,
    max: Option<f32>,
    critical: Option<f32>,
}

pub struct TemperatureModule {
    components: Components,
    state: Option<TemperatureState>,
    chrome: ModuleChrome,
    format: String,
    icons: TemperatureIcons,
    sensor: Option<String>,
    warn_threshold: f32,
    critical_threshold: f32,
    style: TemperatureStyle,
    logged_warning: bool,
}

impl TemperatureModule {
    pub fn new(
        format: Option<String>,
        icons: TemperatureIcons,
        sensor: Option<String>,
        warn_threshold: Option<f32>,
        critical_threshold: Option<f32>,
        chrome: ModuleChrome,
        config: &ModuleConfig,
    ) -> Self {
        Self {
            components: Components::new_with_refreshed_list(),
            state: None,
            chrome,
            format: resolve_format(format),
            icons,
            sensor: normalize_optional_string(sensor),
            warn_threshold: warn_threshold
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(DEFAULT_WARN_THRESHOLD),
            critical_threshold: critical_threshold
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(DEFAULT_CRITICAL_THRESHOLD),
            style: TemperatureStyle::from_config(config),
            logged_warning: false,
        }
    }

    fn icon_text(&self) -> &str {
        match self.state {
            Some(state) if state.current >= self.critical_threshold => &self.icons.critical,
            Some(state) if state.current >= self.warn_threshold => &self.icons.warn,
            Some(_) => &self.icons.normal,
            None => &self.icons.normal,
        }
    }

    fn temp_color(&self) -> Option<Color> {
        let state = self.state?;
        if state.current >= self.critical_threshold {
            Some(self.style.critical_color)
        } else if state.current >= self.warn_threshold {
            Some(self.style.warn_color)
        } else {
            None
        }
    }

    fn render_text(&self) -> String {
        let icon = self.icon_text();
        let (temp, max, critical) = match self.state {
            Some(state) => (
                format!("{:.0}", state.current),
                state
                    .max
                    .map(|v| format!("{:.0}", v))
                    .unwrap_or_else(|| "--".to_string()),
                state
                    .critical
                    .map(|v| format!("{:.0}", v))
                    .unwrap_or_else(|| "--".to_string()),
            ),
            None => ("--".to_string(), "--".to_string(), "--".to_string()),
        };

        self.format
            .replace("{icon}", icon)
            .replace("{temp}", &temp)
            .replace("{max}", &max)
            .replace("{critical}", &critical)
    }

    fn find_temperature(&self) -> Option<TemperatureState> {
        let readings: Vec<SensorReading> = self
            .components
            .iter()
            .map(|component| SensorReading {
                label: component.label().to_ascii_lowercase(),
                current: component.temperature(),
                max: Some(component.max()).filter(|v| v.is_finite()),
                critical: component.critical().filter(|v| v.is_finite()),
            })
            .filter(|reading| reading.current.is_finite())
            .collect();

        let best = match &self.sensor {
            Some(filter) => select_by_filter(&readings, filter),
            None => select_cpu_sensor(&readings),
        }?;

        Some(TemperatureState {
            current: best.current,
            max: best.max,
            critical: best.critical,
        })
    }
}

/// One temperature channel as reported by sysinfo. sysinfo labels hwmon
/// channels as `"<driver> <channel label>"`, e.g. `"k10temp tctl"`,
/// `"coretemp package id 0"` or `"amdgpu junction"`. Labels are lowercased.
#[derive(Debug, Clone)]
struct SensorReading {
    label: String,
    current: f32,
    max: Option<f32>,
    critical: Option<f32>,
}

impl SensorReading {
    /// The channel part of the label when it belongs to `driver`, or `None`.
    fn channel_of(&self, driver: &str) -> Option<&str> {
        let rest = self.label.strip_prefix(driver)?;
        if rest.is_empty() {
            Some("")
        } else {
            rest.strip_prefix(' ')
        }
    }
}

/// CPU hwmon drivers in order of preference, each with the channels that
/// represent the whole CPU (most accurate first). If none of the preferred
/// channels exist, the hottest channel of that driver is used (e.g. the
/// hottest `Core N` on coretemp, or a `Tccd` on k10temp).
const CPU_SENSORS: &[(&str, &[&str])] = &[
    // AMD: Tdie is the real die temperature, Tctl may carry a fan-control offset.
    ("k10temp", &["tdie", "tctl"]),
    ("zenpower", &["tdie", "tctl"]),
    // Intel: the package sensor covers all cores.
    ("coretemp", &["package id"]),
    // ARM SoCs (Raspberry Pi, many SBCs and laptops).
    ("cpu_thermal", &[]),
    ("cpu-thermal", &[]),
    ("soc_thermal", &[]),
];

/// Drivers that are never the CPU. Used only by the last-resort fallback so a
/// busy GPU, SSD or wifi card is not reported as the CPU temperature.
const NON_CPU_DRIVERS: &[&str] = &[
    "amdgpu",
    "radeon",
    "nouveau",
    "nvidia",
    "i915",
    "xe",
    "nvme",
    "drivetemp",
    "iwlwifi",
    "ath",
    "mt79",
    "spd5118",
    "jc42",
    "bat",
];

fn hottest<'a>(readings: impl Iterator<Item = &'a SensorReading>) -> Option<&'a SensorReading> {
    readings.max_by(|a, b| a.current.total_cmp(&b.current))
}

/// Pick the hottest channel whose label contains the user's `sensor` filter.
fn select_by_filter<'a>(readings: &'a [SensorReading], filter: &str) -> Option<&'a SensorReading> {
    let filter = filter.to_ascii_lowercase();
    hottest(readings.iter().filter(|r| r.label.contains(&filter)))
}

/// Pick the reading that best represents the CPU temperature.
fn select_cpu_sensor(readings: &[SensorReading]) -> Option<&SensorReading> {
    for (driver, preferred) in CPU_SENSORS {
        let driver_readings: Vec<(&SensorReading, &str)> = readings
            .iter()
            .filter_map(|r| r.channel_of(driver).map(|channel| (r, channel)))
            .collect();
        if driver_readings.is_empty() {
            continue;
        }

        for channel in *preferred {
            let found = hottest(
                driver_readings
                    .iter()
                    .filter(|(_, c)| c.starts_with(channel))
                    .map(|(r, _)| *r),
            );
            if found.is_some() {
                return found;
            }
        }

        return hottest(driver_readings.iter().map(|(r, _)| *r));
    }

    // No known CPU driver: use the hottest sensor that is not a known
    // non-CPU device (this is usually an ACPI or motherboard CPU zone).
    hottest(
        readings
            .iter()
            .filter(|r| !NON_CPU_DRIVERS.iter().any(|d| r.label.starts_with(d))),
    )
}

impl Module for TemperatureModule {
    fn update_interval(&self) -> std::time::Duration {
        self.chrome
            .update_interval
            .unwrap_or(std::time::Duration::from_secs(1))
    }

    fn update(&mut self) {
        self.components.refresh_list();

        match self.find_temperature() {
            Some(state) => {
                self.state = Some(state);
                self.logged_warning = false;
            }
            None => {
                if !self.logged_warning {
                    let sensor_info = self
                        .sensor
                        .as_deref()
                        .map(|s| format!(" matching '{s}'"))
                        .unwrap_or_default();
                    log::warn!(
                        "No temperature sensor found{}; is lm-sensors / hwmon available?",
                        sensor_info
                    );
                    self.logged_warning = true;
                }
            }
        }
    }

    fn view(&self) -> ModuleView {
        if self.state.is_none() {
            return self.chrome.apply(ModuleView {
                text: prefix_text(self.icon_text(), "N/A"),
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

        let rendered = self.render_text();

        // If temperature exceeds a threshold, colour the entire output.
        let display_style = match self.temp_color() {
            Some(color) => TextStyle {
                color,
                ..base_style.clone()
            },
            None => base_style.clone(),
        };

        self.chrome.apply(ModuleView {
            text: rendered.clone(),
            text_segments: vec![TextSegment {
                text: rendered,
                style: display_style,
            }],
            style: base_style,
            ..Default::default()
        })
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn resolve_format(format: Option<String>) -> String {
    match format {
        Some(format) if !format.trim().is_empty() => format,
        _ => DEFAULT_FORMAT.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ModuleConfig;
    use crate::core::module::{Module, ModuleChrome};

    fn default_module_config() -> ModuleConfig {
        ModuleConfig {
            kind: "temperature".to_string(),
            ..Default::default()
        }
    }

    fn default_chrome() -> ModuleChrome {
        ModuleChrome {
            foreground: None,
            background: None,
            padding: (8.0, 8.0),
            icon_spacing: None,
            update_interval: None,
        }
    }

    fn make_module(format: Option<String>, sensor: Option<String>) -> TemperatureModule {
        TemperatureModule::new(
            format,
            TemperatureIcons::default(),
            sensor,
            None,
            None,
            default_chrome(),
            &default_module_config(),
        )
    }

    #[test]
    fn renders_unavailable_when_no_sensors() {
        let module = make_module(None, None);

        let view = module.view();

        assert!(view.text.contains("N/A"));
    }

    #[test]
    fn renders_temperature_with_state() {
        let mut module = make_module(None, None);
        module.state = Some(TemperatureState {
            current: 45.0,
            max: Some(80.0),
            critical: Some(100.0),
        });

        let view = module.view();

        assert_eq!(view.text, " 45°C");
    }

    #[test]
    fn renders_custom_format() {
        let mut module = make_module(Some("{icon} {temp}°C (max: {max}°C)".to_string()), None);
        module.state = Some(TemperatureState {
            current: 55.0,
            max: Some(90.0),
            critical: None,
        });

        let view = module.view();

        assert_eq!(view.text, " 55°C (max: 90°C)");
    }

    #[test]
    fn applies_warn_color_above_threshold() {
        let mut module = make_module(None, None);
        module.warn_threshold = 60.0;
        module.critical_threshold = 90.0;
        module.state = Some(TemperatureState {
            current: 72.0,
            max: None,
            critical: None,
        });

        let color = module.temp_color();

        assert!(color.is_some());
        assert_eq!(color.unwrap(), WARN_COLOR);
    }

    #[test]
    fn applies_critical_color_above_threshold() {
        let mut module = make_module(None, None);
        module.warn_threshold = 60.0;
        module.critical_threshold = 90.0;
        module.state = Some(TemperatureState {
            current: 95.0,
            max: None,
            critical: None,
        });

        let color = module.temp_color();

        assert!(color.is_some());
        assert_eq!(color.unwrap(), CRITICAL_COLOR);
    }

    #[test]
    fn no_color_below_warn_threshold() {
        let mut module = make_module(None, None);
        module.warn_threshold = 60.0;
        module.critical_threshold = 90.0;
        module.state = Some(TemperatureState {
            current: 45.0,
            max: None,
            critical: None,
        });

        assert!(module.temp_color().is_none());
    }

    #[test]
    fn uses_configured_icons() {
        let mut module = TemperatureModule::new(
            None,
            TemperatureIcons {
                normal: "COOL".to_string(),
                warn: "WARM".to_string(),
                critical: "HOT".to_string(),
            },
            None,
            Some(60.0),
            Some(90.0),
            default_chrome(),
            &default_module_config(),
        );

        module.state = Some(TemperatureState {
            current: 40.0,
            max: None,
            critical: None,
        });
        assert_eq!(module.icon_text(), "COOL");

        module.state = Some(TemperatureState {
            current: 75.0,
            max: None,
            critical: None,
        });
        assert_eq!(module.icon_text(), "WARM");

        module.state = Some(TemperatureState {
            current: 95.0,
            max: None,
            critical: None,
        });
        assert_eq!(module.icon_text(), "HOT");
    }

    #[test]
    fn missing_max_and_critical_render_as_dashes() {
        let mut module = make_module(Some("{temp} max:{max} crit:{critical}".to_string()), None);
        module.state = Some(TemperatureState {
            current: 50.0,
            max: None,
            critical: None,
        });

        assert_eq!(module.view().text, "50 max:-- crit:--");
    }

    #[test]
    fn keeps_user_format_whitespace() {
        let mut module = make_module(Some("  {icon} {temp}°C  ".to_string()), None);
        module.state = Some(TemperatureState {
            current: 42.0,
            max: None,
            critical: None,
        });

        assert_eq!(module.view().text, "   42°C  ");
    }

    fn reading(label: &str, current: f32) -> SensorReading {
        SensorReading {
            label: label.to_ascii_lowercase(),
            current,
            max: None,
            critical: None,
        }
    }

    #[test]
    fn prefers_amd_cpu_over_hotter_gpu() {
        let readings = vec![
            reading("amdgpu junction", 95.0),
            reading("amdgpu edge", 80.0),
            reading("nvme Composite", 60.0),
            reading("k10temp Tctl", 56.0),
            reading("acpitz temp1", 16.0),
        ];

        assert_eq!(select_cpu_sensor(&readings).unwrap().label, "k10temp tctl");
    }

    #[test]
    fn prefers_tdie_over_tctl_and_ccd() {
        let readings = vec![
            reading("k10temp Tctl", 70.0),
            reading("k10temp Tccd1", 75.0),
            reading("k10temp Tdie", 60.0),
        ];

        assert_eq!(select_cpu_sensor(&readings).unwrap().label, "k10temp tdie");
    }

    #[test]
    fn prefers_intel_package_over_cores() {
        let readings = vec![
            reading("coretemp Core 0", 70.0),
            reading("coretemp Package id 0", 65.0),
            reading("nouveau temp1", 90.0),
        ];

        assert_eq!(
            select_cpu_sensor(&readings).unwrap().label,
            "coretemp package id 0"
        );
    }

    #[test]
    fn uses_hottest_core_without_package_sensor() {
        let readings = vec![
            reading("coretemp Core 0", 50.0),
            reading("coretemp Core 1", 58.0),
        ];

        assert_eq!(
            select_cpu_sensor(&readings).unwrap().label,
            "coretemp core 1"
        );
    }

    #[test]
    fn fallback_skips_non_cpu_devices() {
        let readings = vec![
            reading("amdgpu junction", 90.0),
            reading("nvme Composite", 40.0),
            reading("acpitz temp1", 45.0),
        ];

        assert_eq!(select_cpu_sensor(&readings).unwrap().label, "acpitz temp1");
    }

    #[test]
    fn explicit_sensor_filter_overrides_cpu_detection() {
        let readings = vec![
            reading("k10temp Tctl", 56.0),
            reading("amdgpu junction", 47.0),
        ];

        assert_eq!(
            select_by_filter(&readings, "Junction").unwrap().label,
            "amdgpu junction"
        );
    }

    #[test]
    fn driver_prefix_must_match_whole_word() {
        let reading = reading("k10temp_extra Tctl", 50.0);

        assert!(reading.channel_of("k10temp").is_none());
    }
}
