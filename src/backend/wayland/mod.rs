pub mod layer_shell;

use crate::core::bar::Bar;

pub fn run(bars: Vec<Bar>) {
    log::info!("Starting Wayland backend ({} bar(s))", bars.len());

    let mut bars = bars;

    // The first bar runs on the main thread; extras each get their own thread.
    let main_bar = bars.remove(0);
    let handles: Vec<_> = bars
        .into_iter()
        .map(|mut bar| {
            std::thread::spawn(move || {
                layer_shell::run_layer_shell(&mut bar);
            })
        })
        .collect();

    let mut main = main_bar;
    layer_shell::run_layer_shell(&mut main);

    for h in handles {
        let _ = h.join();
    }
}
