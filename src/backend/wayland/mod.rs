pub mod layer_shell;

use crate::core::bar::Bar;

pub fn run(mut bar: Bar) {
    log::info!("Starting Wayland backend");
    layer_shell::run_layer_shell(&mut bar);
}
