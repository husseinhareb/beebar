use sysinfo::System;

use super::{Module, ModuleView};
use crate::renderer::primitives::TextStyle;

pub struct CpuModule {
    sys: System,
    usage: f32,
}

impl CpuModule {
    pub fn new() -> Self {
        Self {
            sys: System::new(),
            usage: 0.0,
        }
    }
}

impl Module for CpuModule {
    fn update(&mut self) {
        self.sys.refresh_cpu_all();
        let cpus = self.sys.cpus();
        if !cpus.is_empty() {
            self.usage = cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32;
        }
    }

    fn view(&self) -> ModuleView {
        ModuleView {
            text: format!("CPU {:.0}%", self.usage),
            style: TextStyle::default(),
            ..Default::default()
        }
    }
}
