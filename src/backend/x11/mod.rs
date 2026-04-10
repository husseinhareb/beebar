pub mod window;

use crate::core::bar::Bar;

pub fn run(mut bar: Bar) {
    log::info!("Starting X11 backend");
    window::run_x11(&mut bar);
}
