#[cfg(feature = "wayland")]
pub mod wayland;

#[cfg(feature = "x11")]
pub mod x11;

use crate::core::bar::Bar;

/// Detect the current display server and return the appropriate backend.
pub fn detect_backend() -> BackendKind {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        BackendKind::Wayland
    } else if std::env::var("DISPLAY").is_ok() {
        BackendKind::X11
    } else {
        panic!("No display server detected (neither WAYLAND_DISPLAY nor DISPLAY is set)");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Wayland,
    X11,
}

/// Create and run all bars with the appropriate backend.
pub fn run(bars: Vec<Bar>, kind: BackendKind) {
    match kind {
        #[cfg(feature = "wayland")]
        BackendKind::Wayland => wayland::run(bars),
        #[cfg(feature = "x11")]
        BackendKind::X11 => x11::run(bars),
        #[allow(unreachable_patterns)]
        _ => panic!("Backend {:?} not compiled in (enable the feature)", kind),
    }
}
