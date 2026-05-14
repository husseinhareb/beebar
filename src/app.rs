use crate::core::bar::Bar;
use crate::core::config::Config;
use crate::core::config::BarConfig;
use crate::core::config::ModuleConfig;
use crate::core::layout::Alignment;
use crate::core::module::ModuleChrome;
use crate::core::module::battery::BatteryIcons;
use crate::core::module::battery::BatteryModule;
use crate::core::module::bluetooth::BluetoothModule;
use crate::core::module::brightness::BrightnessIcons;
use crate::core::module::brightness::BrightnessModule;
use crate::core::module::clock::ClockModule;
use crate::core::module::cpu::CpuModule;
use crate::core::module::custom::CustomModule;
use crate::core::module::memory::MemoryModule;
use crate::core::module::network::NetworkModule;
use crate::core::module::playback::{PlaybackButtonIcons, PlaybackModule};
use crate::core::module::temperature::TemperatureIcons;
use crate::core::module::temperature::TemperatureModule;
use crate::core::module::tray::TrayModule;
use crate::core::module::volume::VolumeIcons;
use crate::core::module::volume::VolumeModule;
use crate::core::module::window::WindowModule;
use crate::core::module::workspaces::WorkspacesModule;
use crate::renderer::primitives::TextStyle;
use std::collections::HashSet;
use std::time::Duration;

fn build_module(
    name: &str,
    mcfg: &ModuleConfig,
    default_padding: (f64, f64),
) -> Option<Box<dyn crate::core::module::Module>> {
    let chrome = ModuleChrome::from_config(mcfg, default_padding);

    match mcfg.kind.as_str() {
        "brightness" => Some(Box::new(BrightnessModule::new(
            mcfg.backend.clone(),
            mcfg.device.clone(),
            mcfg.slider_width,
            mcfg.max_brightness,
            chrome,
            BrightnessIcons::from_config(mcfg),
            crate::core::module::SliderGlyphs::from_config(mcfg),
            mcfg,
        ))),
        "clock" => {
            let fmt = mcfg.format.clone().unwrap_or_else(|| "%H:%M:%S".into());
            Some(Box::new(ClockModule::new(fmt, chrome)))
        }
        "cpu" => Some(Box::new(CpuModule::new(chrome, mcfg.icon.clone()))),
        "memory" => Some(Box::new(MemoryModule::new(
            mcfg.format.clone(),
            mcfg.icon.clone(),
            chrome,
        ))),
        "playback" => Some(Box::new(PlaybackModule::new(
            mcfg.format.clone(),
            mcfg.icon.clone(),
            mcfg.icon_unavailable.clone(),
            chrome,
            PlaybackButtonIcons::from_config(mcfg),
        ))),
        "battery" => Some(Box::new(BatteryModule::new(
            BatteryIcons::from_config(mcfg),
            chrome,
            mcfg.device.clone(),
        ))),
        "temperature" => Some(Box::new(TemperatureModule::new(
            mcfg.format.clone(),
            TemperatureIcons::from_config(mcfg),
            mcfg.sensor.clone(),
            mcfg.warn_threshold,
            mcfg.critical_threshold,
            chrome,
            mcfg,
        ))),
        "network" => Some(Box::new(NetworkModule::new(
            mcfg.interface.clone(),
            mcfg.format.clone(),
            chrome,
        ))),
        "custom" => {
            let cmd = mcfg.command.clone().unwrap_or_else(|| "echo ???".into());
            Some(Box::new(CustomModule::new(cmd, chrome)))
        }
        "workspaces" => {
            let count = mcfg.count.unwrap_or_else(|| {
                mcfg.labels
                    .as_ref()
                    .map(|labels| labels.len() as u32)
                    .filter(|count| *count > 0)
                    .unwrap_or(10)
            });
            Some(Box::new(WorkspacesModule::new(
                count,
                mcfg.labels.clone().unwrap_or_default(),
                chrome,
                mcfg,
            )))
        }
        "tray" => {
            let size = mcfg.icon_size.unwrap_or(22);
            Some(Box::new(TrayModule::new(size, chrome)))
        }
        "window" => Some(Box::new(WindowModule::new(
            mcfg.backend.clone(),
            mcfg.max_length,
            mcfg.empty_label.clone(),
            chrome,
        ))),
        "bluetooth" => Some(Box::new(BluetoothModule::new(
            mcfg.format.clone(),
            mcfg.icon_on.clone(),
            mcfg.icon_off.clone(),
            mcfg.icon_no_controller.clone(),
            chrome,
            Duration::from_millis(mcfg.poll_interval_ms.unwrap_or(2000)),
        ))),
        "volume" => Some(Box::new(VolumeModule::new(
            mcfg.backend.clone(),
            mcfg.device.clone(),
            mcfg.slider_width,
            mcfg.max_volume,
            chrome,
            VolumeIcons::from_config(mcfg),
            crate::core::module::SliderGlyphs::from_config(mcfg),
            mcfg,
        ))),
        other => {
            log::warn!("Unknown module type '{}' for '{}'", other, name);
            None
        }
    }
}

fn add_grouped_modules(
    bar: &mut Bar,
    config: &Config,
    groups: &[Vec<String>],
    alignment: Alignment,
    default_padding: (f64, f64),
    placed: &mut HashSet<String>,
) {
    for group in groups {
        let mut accepted = Vec::with_capacity(group.len());
        for name in group {
            if !placed.insert(name.clone()) {
                log::warn!(
                    "Module '{}' is listed more than once in section ordering; skipping duplicate",
                    name
                );
                continue;
            }

            let Some(mcfg) = config.module.get(name) else {
                log::warn!(
                    "Module '{}' is listed in the bar layout but has no [module.{}] config",
                    name,
                    name
                );
                continue;
            };

            let Some(module) = build_module(name, mcfg, default_padding) else {
                continue;
            };

            bar.add_module(name.clone(), module);
            accepted.push(name.clone());
        }
        bar.add_group(accepted, alignment);
    }
}

/// Build a `Bar` from the loaded configuration.
pub fn build_bar(name: &str, bar_config: &BarConfig, config: &Config) -> Bar {
    let mut bar = Bar::new(0, bar_config.height);
    bar.name = name.to_string();
    bar.bottom = bar_config.is_bottom();
    bar.text_y_offset = bar_config.text_y_offset;
    let default_padding = bar_config.resolved_padding();
    bar.background = bar_config.resolved_background_color();
    bar.margin_top = bar_config.margin_top.max(0.0);
    bar.margin_bottom = bar_config.margin_bottom.max(0.0);
    bar.margin_left = bar_config.margin_left.max(0.0);
    bar.margin_right = bar_config.margin_right.max(0.0);
    bar.group_spacing = bar_config.group_spacing.max(0.0);
    bar.corner_radius = bar_config.corner_radius.max(0.0);
    bar.group_background = bar_config.resolved_module_background();
    bar.refresh_interval = bar_config.refresh_interval();
    bar.text_style = TextStyle {
        color: bar_config.resolved_foreground_color(),
        font_family: bar_config.resolved_font_family(),
        font_size: bar_config.resolved_font_size(),
        bold: bar_config.bold,
        ..TextStyle::default()
    };

    log::info!(
        "[bar/{}] font='{}' size={}px height={} position={}",
        name,
        bar.text_style.font_family,
        bar.text_style.font_size,
        bar_config.height,
        bar_config.position,
    );

    let mut placed = HashSet::new();

    let groups_left = bar_config.effective_groups_left();
    let groups_center = bar_config.effective_groups_center();
    let groups_right = bar_config.effective_groups_right();

    add_grouped_modules(
        &mut bar,
        config,
        &groups_left,
        Alignment::Left,
        default_padding,
        &mut placed,
    );
    add_grouped_modules(
        &mut bar,
        config,
        &groups_center,
        Alignment::Center,
        default_padding,
        &mut placed,
    );
    add_grouped_modules(
        &mut bar,
        config,
        &groups_right,
        Alignment::Right,
        default_padding,
        &mut placed,
    );

    bar
}

/// Build all bars defined in the config.
pub fn build_bars(config: &Config) -> Vec<Bar> {
    let mut bars = Vec::new();
    // Sort by name for deterministic ordering.
    let mut names: Vec<&String> = config.bar.keys().collect();
    names.sort();
    for name in names {
        let bar_config = &config.bar[name];
        bars.push(build_bar(name, bar_config, config));
    }
    if bars.is_empty() {
        log::warn!("No [bar.*] sections found in config; nothing to display");
    }
    bars
}

#[cfg(test)]
mod tests {
    use super::build_bars;
    use crate::core::config::Config;

    #[test]
    fn preserves_module_order_from_section_lists() {
        let config: Config = toml::from_str(
            r#"
[bar.main]
height = 30
modules_right = ["brightness", "battery", "volume", "cpu"]

[module.cpu]
type = "cpu"

[module.volume]
type = "volume"

[module.battery]
type = "battery"

[module.brightness]
type = "brightness"
"#,
        )
        .expect("test config should parse");

        let bars = build_bars(&config);
        assert_eq!(bars.len(), 1);
        let bar = &bars[0];

        // Flat `modules_right` becomes one single-module group per entry,
        // preserving order. Each inner Vec has exactly one module id.
        assert_eq!(
            bar.layout.right,
            vec![
                vec!["brightness".to_string()],
                vec!["battery".to_string()],
                vec!["volume".to_string()],
                vec!["cpu".to_string()],
            ]
        );
    }

    #[test]
    fn explicit_groups_take_precedence_over_flat_modules() {
        let config: Config = toml::from_str(
            r#"
[bar.main]
height = 30
modules_right = ["cpu"]
groups_right = [["brightness", "battery"], ["volume"]]

[module.cpu]
type = "cpu"

[module.volume]
type = "volume"

[module.battery]
type = "battery"

[module.brightness]
type = "brightness"
"#,
        )
        .expect("test config should parse");

        let bars = build_bars(&config);
        let bar = &bars[0];

        assert_eq!(
            bar.layout.right,
            vec![
                vec!["brightness".to_string(), "battery".to_string()],
                vec!["volume".to_string()],
            ]
        );
    }
}
