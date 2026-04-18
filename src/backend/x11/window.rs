use x11rb::COPY_DEPTH_FROM_PARENT;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::core::bar::Bar;
use crate::renderer::cairo_renderer::CairoRenderer;
use crate::renderer::color::Color;
use crate::renderer::primitives::{Point, Rect, Renderer};

/// EWMH atoms needed for dock behavior.
struct Atoms {
    wm_window_type: Atom,
    wm_window_type_dock: Atom,
    wm_strut_partial: Atom,
    wm_state: Atom,
    wm_state_sticky: Atom,
    wm_state_above: Atom,
}

impl Atoms {
    fn intern(conn: &RustConnection) -> Self {
        let wm_window_type = Self::atom(conn, b"_NET_WM_WINDOW_TYPE");
        let wm_window_type_dock = Self::atom(conn, b"_NET_WM_WINDOW_TYPE_DOCK");
        let wm_strut_partial = Self::atom(conn, b"_NET_WM_STRUT_PARTIAL");
        let wm_state = Self::atom(conn, b"_NET_WM_STATE");
        let wm_state_sticky = Self::atom(conn, b"_NET_WM_STATE_STICKY");
        let wm_state_above = Self::atom(conn, b"_NET_WM_STATE_ABOVE");
        Self {
            wm_window_type,
            wm_window_type_dock,
            wm_strut_partial,
            wm_state,
            wm_state_sticky,
            wm_state_above,
        }
    }

    fn atom(conn: &RustConnection, name: &[u8]) -> Atom {
        conn.intern_atom(false, name)
            .expect("intern_atom request failed")
            .reply()
            .expect("intern_atom reply failed")
            .atom
    }
}

pub fn run_x11(bar: &mut Bar) {
    let (conn, screen_num) = RustConnection::connect(None).expect("Failed to connect to X server");
    let screen = &conn.setup().roots[screen_num];

    let width = screen.width_in_pixels as u32;
    let height = bar.height;
    bar.width = width;

    let win = conn.generate_id().unwrap();
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        win,
        screen.root,
        0,
        0,
        width as u16,
        height as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new()
            .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS)
            .override_redirect(1)
            .background_pixel(screen.black_pixel),
    )
    .unwrap();

    // Set EWMH properties for dock behavior
    let atoms = Atoms::intern(&conn);

    conn.change_property32(
        PropMode::REPLACE,
        win,
        atoms.wm_window_type,
        AtomEnum::ATOM,
        &[atoms.wm_window_type_dock],
    )
    .unwrap();

    conn.change_property32(
        PropMode::REPLACE,
        win,
        atoms.wm_state,
        AtomEnum::ATOM,
        &[atoms.wm_state_sticky, atoms.wm_state_above],
    )
    .unwrap();

    // Strut: reserve space at the top
    // _NET_WM_STRUT_PARTIAL: left, right, top, bottom,
    //   left_start_y, left_end_y, right_start_y, right_end_y,
    //   top_start_x, top_end_x, bottom_start_x, bottom_end_x
    let strut = [0u32, 0, height, 0, 0, 0, 0, 0, 0, width, 0, 0];
    conn.change_property32(
        PropMode::REPLACE,
        win,
        atoms.wm_strut_partial,
        AtomEnum::CARDINAL,
        &strut,
    )
    .unwrap();

    conn.map_window(win).unwrap();
    conn.flush().unwrap();

    // Create GC for drawing
    let gc = conn.generate_id().unwrap();
    conn.create_gc(gc, win, &CreateGCAux::new()).unwrap();

    let mut renderer = CairoRenderer::new(width, height);

    loop {
        let event = conn.wait_for_event().expect("X11 event error");
        match event {
            Event::Expose(_) => {
                bar.update_all();
                render_bar(bar, &mut renderer, width, height);

                // Put image data onto the window
                let data = renderer.data();
                conn.put_image(
                    ImageFormat::Z_PIXMAP,
                    win,
                    gc,
                    width as u16,
                    height as u16,
                    0,
                    0,
                    0,
                    screen.root_depth,
                    data,
                )
                .unwrap();
                conn.flush().unwrap();
            }
            Event::ButtonPress(ev) => {
                use crate::core::event::{ClickEvent, MouseButton};
                let button = match ev.detail {
                    1 => MouseButton::Left,
                    2 => MouseButton::Middle,
                    3 => MouseButton::Right,
                    n => MouseButton::Other(n as u32),
                };
                let click = ClickEvent {
                    x: ev.event_x as f64,
                    module_width: 0.0,
                    y: ev.event_y as f64,
                    button,
                };

                let modules = &bar.modules;
                let measure = |id: &String| -> f64 {
                    if let Some(m) = modules.get(id) {
                        let view = m.view();
                        if !view.icons.is_empty() {
                            let icon_size = height.saturating_sub(4) as f64;
                            let n = view.icons.len() as f64;
                            view.padding.0
                                + view.padding.1
                                + n * icon_size
                                + (n - 1.0).max(0.0) * view.icon_spacing
                        } else {
                            view.text_width(&renderer) + view.padding.0 + view.padding.1
                        }
                    } else {
                        0.0
                    }
                };
                let regions = bar.layout.compute(width as f64, &measure);
                bar.handle_click(&regions, &click);

                // Trigger redraw
                conn.send_event(
                    false,
                    win,
                    EventMask::EXPOSURE,
                    ExposeEvent {
                        response_type: 12,
                        sequence: 0,
                        window: win,
                        x: 0,
                        y: 0,
                        width: width as u16,
                        height: height as u16,
                        count: 0,
                    },
                )
                .unwrap();
                conn.flush().unwrap();
            }
            _ => {}
        }
    }
}

fn render_bar(bar: &Bar, renderer: &mut CairoRenderer, width: u32, height: u32) {
    renderer.begin(width, height);

    // Background
    let bg_color = Color::from_hex("#1e1e2e").unwrap_or(Color::BLACK);
    renderer.draw_rect(
        Rect {
            x: 0.0,
            y: 0.0,
            width: width as f64,
            height: height as f64,
        },
        bg_color,
    );

    let modules = &bar.modules;
    let measure = |id: &String| -> f64 {
        if let Some(m) = modules.get(id) {
            let view = m.view();
            if !view.icons.is_empty() {
                let icon_size = height.saturating_sub(4) as f64;
                let n = view.icons.len() as f64;
                view.padding.0
                    + view.padding.1
                    + n * icon_size
                    + (n - 1.0).max(0.0) * view.icon_spacing
            } else {
                view.text_width(renderer) + view.padding.0 + view.padding.1
            }
        } else {
            0.0
        }
    };

    let regions = bar.layout.compute(width as f64, &measure);

    for region in &regions {
        if let Some(module) = modules.get(&region.id) {
            let view = module.view();

            if let Some(bg) = view.background {
                renderer.draw_rect(
                    Rect {
                        x: region.x,
                        y: 0.0,
                        width: region.width,
                        height: height as f64,
                    },
                    bg,
                );
            }

            if !view.icons.is_empty() {
                let icon_size = height.saturating_sub(4);
                let mut ix = region.x + view.padding.0;
                let iy = ((height as f64 - icon_size as f64) / 2.0).max(0.0);
                for icon_data in &view.icons {
                    renderer.draw_icon(
                        Point { x: ix, y: iy },
                        &icon_data.pixels,
                        icon_data.width,
                        icon_data.height,
                        icon_size,
                    );
                    ix += icon_size as f64 + view.icon_spacing;
                }
            } else {
                let y = (height as f64 - view.text_height()) / 2.0;
                let mut x = region.x + view.padding.0;
                if view.text_segments.is_empty() {
                    renderer.draw_text(Point { x, y }, &view.text, &view.style);
                } else {
                    for segment in &view.text_segments {
                        x += renderer.draw_text(Point { x, y }, &segment.text, &segment.style);
                    }
                }
            }
        }
    }

    renderer.end();
}
