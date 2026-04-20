use crate::core::bar::Bar;
use crate::core::config::Config;
use crate::core::config::ModuleConfig;
use crate::core::layout::Alignment;
use crate::core::module::ModuleChrome;
use crate::core::module::battery::BatteryIcons;
use crate::core::module::battery::BatteryModule;
use crate::core::module::brightness::BrightnessIcons;
use crate::core::module::brightness::BrightnessModule;
use crate::core::module::clock::ClockModule;
use crate::core::module::cpu::CpuModule;
use crate::core::module::custom::CustomModule;
use crate::core::module::memory::MemoryModule;
use crate::core::module::network::NetworkModule;
use crate::core::module::tray::TrayModule;
use crate::core::module::volume::VolumeIcons;
use crate::core::module::volume::VolumeModule;
use crate::core::module::workspaces::WorkspacesModule;
use crate::renderer::primitives::TextStyle;
use std::collections::HashSet;

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
        "battery" => Some(Box::new(BatteryModule::new(
            BatteryIcons::from_config(mcfg),
            chrome,
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

fn add_ordered_modules(
    bar: &mut Bar,
    config: &Config,
    names: &[String],
    alignment: Alignment,
    default_padding: (f64, f64),
    placed: &mut HashSet<String>,
) {
    for name in names {
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

        bar.add_module(name.clone(), module, alignment);
    }
}

/// Build a `Bar` from the loaded configuration.
pub fn build_bar(config: &Config) -> Bar {
    let mut bar = Bar::new(1920, config.height);
    let default_padding = config.resolved_padding();
    bar.background = config.resolved_background_color();
    bar.text_style = TextStyle {
        color: config.resolved_foreground_color(),
        font_family: config.resolved_font_family(),
        font_size: config.resolved_font_size(),
        ..TextStyle::default()
    };

    log::info!(
        "Using bar font '{}' at {}px",
        bar.text_style.font_family,
        bar.text_style.font_size
    );

    let mut placed = HashSet::new();

    add_ordered_modules(
        &mut bar,
        config,
        &config.modules_left,
        Alignment::Left,
        default_padding,
        &mut placed,
    );
    add_ordered_modules(
        &mut bar,
        config,
        &config.modules_center,
        Alignment::Center,
        default_padding,
        &mut placed,
    );
    add_ordered_modules(
        &mut bar,
        config,
        &config.modules_right,
        Alignment::Right,
        default_padding,
        &mut placed,
    );

    for (name, mcfg) in &config.module {
        if placed.contains(name) {
            continue;
        }

        log::warn!(
            "Module '{}' not placed in any section, defaulting to left",
            name
        );

        let Some(module) = build_module(name, mcfg, default_padding) else {
            continue;
        };

        bar.add_module(name.clone(), module, Alignment::Left);
    }

    bar
}

#[cfg(test)]
mod tests {
    use super::build_bar;
    use crate::core::config::Config;

    #[test]
    fn preserves_module_order_from_section_lists() {
        let config: Config = toml::from_str(
            r#"
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

        let bar = build_bar(&config);

        assert_eq!(
            bar.layout.right,
            vec![
                "brightness".to_string(),
                "battery".to_string(),
                "volume".to_string(),
                "cpu".to_string()
            ]
        );
    }
}
