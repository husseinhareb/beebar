use crate::core::bar::Bar;
use crate::core::config::Config;
use crate::core::layout::Alignment;
use crate::core::module::battery::BatteryModule;
use crate::core::module::clock::ClockModule;
use crate::core::module::cpu::CpuModule;
use crate::core::module::custom::CustomModule;

/// Build a `Bar` from the loaded configuration.
pub fn build_bar(config: &Config) -> Bar {
    let mut bar = Bar::new(1920, config.height);

    // Instantiate modules from config
    for (name, mcfg) in &config.module {
        let module: Box<dyn crate::core::module::Module> = match mcfg.kind.as_str() {
            "clock" => {
                let fmt = mcfg.format.clone().unwrap_or_else(|| "%H:%M:%S".into());
                Box::new(ClockModule::new(fmt))
            }
            "cpu" => Box::new(CpuModule::new()),
            "battery" => Box::new(BatteryModule::new()),
            "custom" => {
                let cmd = mcfg
                    .command
                    .clone()
                    .unwrap_or_else(|| "echo ???".into());
                let interval = mcfg.interval.unwrap_or(5000);
                Box::new(CustomModule::new(cmd, interval))
            }
            other => {
                log::warn!("Unknown module type '{}' for '{}'", other, name);
                continue;
            }
        };

        let alignment = if config.modules_left.contains(name) {
            Alignment::Left
        } else if config.modules_center.contains(name) {
            Alignment::Center
        } else if config.modules_right.contains(name) {
            Alignment::Right
        } else {
            log::warn!(
                "Module '{}' not placed in any section, defaulting to left",
                name
            );
            Alignment::Left
        };

        bar.add_module(name.clone(), module, alignment);
    }

    bar
}
