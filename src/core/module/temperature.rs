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
        let matching = self.components.iter().filter(|component| {
            match &self.sensor {
                Some(filter) => component
                    .label()
                    .to_ascii_lowercase()
                    .contains(&filter.to_ascii_lowercase()),
                None => true,
            }
        });

        // Pick the component with the highest current temperature among matches,
        // which is typically the most relevant reading (e.g. CPU package temp).
        let best = matching.max_by(|a, b| {
            let ta = a.temperature();
            let tb = b.temperature();
            ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal)
        })?;

        let current = best.temperature();
        Some(TemperatureState {
            current,
            max: Some(best.max()),
            critical: best.critical(),
        })
    }
}

impl Module for TemperatureModule {
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
        let mut module = make_module(
            Some("{icon} {temp}°C (max: {max}°C)".to_string()),
            None,
        );
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
        let mut module = make_module(
            Some("{temp} max:{max} crit:{critical}".to_string()),
            None,
        );
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
}
