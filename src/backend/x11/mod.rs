pub mod window;

use crate::core::bar::Bar;

pub fn run(bars: Vec<Bar>) {
    log::info!("Starting X11 backend ({} bar(s))", bars.len());

    let mut bars = bars;

    // The first bar runs on the main thread; extras each get their own thread.
    let main_bar = bars.remove(0);
    let handles: Vec<_> = bars
        .into_iter()
        .map(|mut bar| {
            std::thread::spawn(move || {
                window::run_x11(&mut bar);
            })
        })
        .collect();

    let mut main = main_bar;
    window::run_x11(&mut main);

    for h in handles {
        let _ = h.join();
    }
}
