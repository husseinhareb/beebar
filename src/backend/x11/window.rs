use std::thread;
use std::time::{Duration, Instant};

use x11rb::COPY_DEPTH_FROM_PARENT;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::core::bar::{Bar, render_bar};
use crate::core::layout::BarLayout;
use crate::core::module::ModuleId;
use crate::core::popup::{POPUP_GAP, PopupLayout, PopupMenu, draw_popup, layout_popup};
use crate::renderer::cairo_renderer::CairoRenderer;
use crate::renderer::primitives::Renderer;

/// EWMH atoms needed for dock behavior.
struct Atoms {
    wm_window_type: Atom,
    wm_window_type_dock: Atom,
    wm_strut: Atom,
    wm_strut_partial: Atom,
    wm_state: Atom,
    wm_state_sticky: Atom,
    wm_state_above: Atom,
}

impl Atoms {
    fn intern(conn: &RustConnection) -> Self {
        let wm_window_type = Self::atom(conn, b"_NET_WM_WINDOW_TYPE");
        let wm_window_type_dock = Self::atom(conn, b"_NET_WM_WINDOW_TYPE_DOCK");
        let wm_strut = Self::atom(conn, b"_NET_WM_STRUT");
        let wm_strut_partial = Self::atom(conn, b"_NET_WM_STRUT_PARTIAL");
        let wm_state = Self::atom(conn, b"_NET_WM_STATE");
        let wm_state_sticky = Self::atom(conn, b"_NET_WM_STATE_STICKY");
        let wm_state_above = Self::atom(conn, b"_NET_WM_STATE_ABOVE");
        Self {
            wm_window_type,
            wm_window_type_dock,
            wm_strut,
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

struct PopupWindow {
    win: Window,
    gc: Gcontext,
    owner: ModuleId,
    menu: PopupMenu,
    layout: PopupLayout,
    renderer: CairoRenderer,
    hovered_item: Option<usize>,
}

pub fn run_x11(bar: &mut Bar) {
    let (conn, screen_num) = RustConnection::connect(None).expect("Failed to connect to X server");
    let screen = &conn.setup().roots[screen_num];

    let screen_width = screen.width_in_pixels as u32;
    let screen_height = screen.height_in_pixels as u32;
    let width = screen_width;
    let height = bar.height;
    bar.width = width;

    // Place bar at top or bottom of the screen.
    let y_pos: i16 = if bar.bottom {
        (screen_height - height) as i16
    } else {
        0
    };

    let win = conn.generate_id().unwrap();
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        win,
        screen.root,
        0,
        y_pos,
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

    // Strut: reserve space at the top or bottom
    let strut = if bar.bottom {
        [0u32, 0, 0, height]
    } else {
        [0u32, 0, height, 0]
    };
    conn.change_property32(
        PropMode::REPLACE,
        win,
        atoms.wm_strut,
        AtomEnum::CARDINAL,
        &strut,
    )
    .unwrap();

    // _NET_WM_STRUT_PARTIAL: left, right, top, bottom,
    //   left_start_y, left_end_y, right_start_y, right_end_y,
    //   top_start_x, top_end_x, bottom_start_x, bottom_end_x
    let strut_partial = if bar.bottom {
        [
            0u32,
            0,
            0,
            height,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            width.saturating_sub(1),
        ]
    } else {
        [
            0u32,
            0,
            height,
            0,
            0,
            0,
            0,
            0,
            0,
            width.saturating_sub(1),
            0,
            0,
        ]
    };
    conn.change_property32(
        PropMode::REPLACE,
        win,
        atoms.wm_strut_partial,
        AtomEnum::CARDINAL,
        &strut_partial,
    )
    .unwrap();

    conn.map_window(win).unwrap();
    conn.flush().unwrap();

    // Create GC for drawing
    let gc = conn.generate_id().unwrap();
    conn.create_gc(gc, win, &CreateGCAux::new()).unwrap();

    let mut renderer = CairoRenderer::new(width, height);
    let mut popup: Option<PopupWindow> = None;
    // Upper bound on time between ticks: even if no module is due, redraw
    // this often so background-worker modules (tray, bluetooth, window-via-
    // inotify) get their pushed state on screen.
    let refresh_ceiling = bar.refresh_interval;
    let idle_sleep = Duration::from_millis(16);
    let mut next_tick = Instant::now();
    let mut needs_redraw = true;

    loop {
        while let Some(event) = conn.poll_for_event().expect("X11 event error") {
            match event {
                Event::Expose(_) => {
                    needs_redraw = true;
                }
                Event::ButtonPress(ev) => {
                    use crate::core::event::{ClickEvent, MouseButton};
                    // X11 reports scroll wheel motion as button presses on
                    // detail 4 (up) and 5 (down).
                    let button = match ev.detail {
                        1 => MouseButton::Left,
                        2 => MouseButton::Middle,
                        3 => MouseButton::Right,
                        4 => MouseButton::ScrollUp,
                        5 => MouseButton::ScrollDown,
                        n => MouseButton::Other(n as u32),
                    };

                    if let Some(active_popup) = popup.as_ref() {
                        if ev.event == active_popup.win {
                            // The pointer is grabbed on the popup window, so
                            // every click outside of it is reported here too
                            // (with negative or out-of-range coordinates).
                            // hit_test() rejects those, dismissing the menu.
                            if let Some(item_idx) = active_popup.layout.hit_test(
                                &active_popup.menu.items,
                                ev.event_x as f64,
                                ev.event_y as f64,
                            ) {
                                bar.handle_popup_click(&active_popup.owner, item_idx, button);
                            } else {
                                bar.dismiss_all_popups();
                            }
                            bar.update_all();
                            needs_redraw = true;
                            continue;
                        }
                    }

                    if popup.is_some() {
                        bar.dismiss_all_popups();
                    }

                    let click = ClickEvent {
                        x: ev.event_x as f64,
                        bar_x: ev.event_x as f64,
                        screen_x: ev.root_x as f64,
                        module_width: 0.0,
                        y: ev.event_y as f64,
                        bar_y: ev.event_y as f64,
                        screen_y: ev.root_y as f64,
                        button,
                    };

                    let groups = bar.compute_groups(width as f64, &renderer);
                    let regions = BarLayout::flatten_modules(&groups);
                    bar.handle_click(&regions, &click);
                    bar.update_all();
                    needs_redraw = true;
                }
                _ => {}
            }
        }

        // Drain any MotionNotify events for the popup (they can queue up fast
        // so only act on the last one between frames).
        let mut last_motion: Option<(i16, i16, Window)> = None;
        while let Some(event) = conn.poll_for_event().expect("X11 event error") {
            if let Event::MotionNotify(ev) = event {
                last_motion = Some((ev.event_x, ev.event_y, ev.event));
            }
        }
        if let Some((mx, my, event_win)) = last_motion {
            if let Some(p) = popup.as_mut() {
                if event_win == p.win {
                    let new_hover =
                        p.layout.hit_test(&p.menu.items, mx as f64, my as f64);
                    if new_hover != p.hovered_item {
                        p.hovered_item = new_hover;
                        render_popup_window(&conn, p, &bar.text_style, screen.root_depth);
                        conn.flush().unwrap();
                    }
                }
            }
        }

        let now = Instant::now();
        if now >= next_tick {
            let tick = bar.tick();
            let ceiling = now + refresh_ceiling;
            next_tick = match tick.next_due {
                Some(due) => due.min(ceiling),
                None => ceiling,
            };
            // Always redraw on the ceiling tick so background-worker state
            // (tray icons, bluetooth, inotify-pushed window titles) shows up.
            needs_redraw = true;
            let _ = tick.updated; // currently always-redraw; reserved for later
        }

        if needs_redraw {
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
            sync_popup_window(
                &conn,
                screen,
                bar,
                &renderer,
                &mut popup,
                y_pos,
                screen.root_depth,
            );
            conn.flush().unwrap();
            needs_redraw = false;
        }

        let sleep_for = next_tick
            .saturating_duration_since(Instant::now())
            .min(idle_sleep);
        if !sleep_for.is_zero() {
            thread::sleep(sleep_for);
        }
    }
}

fn sync_popup_window(
    conn: &RustConnection,
    screen: &Screen,
    bar: &Bar,
    text_renderer: &CairoRenderer,
    popup_state: &mut Option<PopupWindow>,
    bar_y: i16,
    depth: u8,
) {
    let active_popup = bar.active_popup();

    match active_popup {
        Some((owner, menu)) => {
            let Some(layout) =
                layout_popup(&menu, bar.width as f64, &bar.text_style, text_renderer)
            else {
                if let Some(popup) = popup_state.take() {
                    destroy_popup_window(conn, popup);
                }
                return;
            };

            let popup_width = layout.pixel_width();
            let popup_height = layout.pixel_height();
            let popup_x = layout.x.round() as i32;
            let popup_y = if bar.bottom {
                i32::from(bar_y) - popup_height as i32 - POPUP_GAP.round() as i32
            } else {
                i32::from(bar_y) + bar.height as i32 + POPUP_GAP.round() as i32
            };

            if popup_state.is_none() {
                let win = conn.generate_id().unwrap();
                conn.create_window(
                    COPY_DEPTH_FROM_PARENT,
                    win,
                    screen.root,
                    popup_x as i16,
                    popup_y as i16,
                    popup_width as u16,
                    popup_height as u16,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    0,
                    &CreateWindowAux::new()
                        .event_mask(
                            EventMask::EXPOSURE
                                | EventMask::BUTTON_PRESS
                                | EventMask::POINTER_MOTION,
                        )
                        .override_redirect(1)
                        .background_pixel(screen.black_pixel),
                )
                .unwrap();

                let gc = conn.generate_id().unwrap();
                conn.create_gc(gc, win, &CreateGCAux::new()).unwrap();
                conn.map_window(win).unwrap();

                // Actively grab the pointer so we receive every button press,
                // even those outside the popup window. Out-of-range clicks
                // are routed to `win` with coordinates outside the popup
                // bounds, which the click handler treats as a dismiss.
                let _ = conn.grab_pointer(
                    false,
                    win,
                    EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    x11rb::NONE,
                    x11rb::NONE,
                    x11rb::CURRENT_TIME,
                );

                *popup_state = Some(PopupWindow {
                    win,
                    gc,
                    owner: owner.clone(),
                    menu: menu.clone(),
                    layout: layout.clone(),
                    renderer: CairoRenderer::new(popup_width, popup_height),
                    hovered_item: None,
                });
            }

            let popup = popup_state.as_mut().unwrap();
            popup.owner = owner;
            popup.menu = menu;
            popup.layout = layout;
            conn.configure_window(
                popup.win,
                &ConfigureWindowAux::new()
                    .x(popup_x)
                    .y(popup_y)
                    .width(popup_width)
                    .height(popup_height)
                    .stack_mode(StackMode::ABOVE),
            )
            .unwrap();
            render_popup_window(conn, popup, &bar.text_style, depth);
            conn.map_window(popup.win).ok();
        }
        None => {
            if let Some(popup) = popup_state.take() {
                destroy_popup_window(conn, popup);
            }
        }
    }
}

fn destroy_popup_window(conn: &RustConnection, popup: PopupWindow) {
    let _ = conn.ungrab_pointer(x11rb::CURRENT_TIME);
    let _ = conn.free_gc(popup.gc);
    let _ = conn.destroy_window(popup.win);
}

fn render_popup_window(
    conn: &RustConnection,
    popup: &mut PopupWindow,
    text_style: &crate::renderer::primitives::TextStyle,
    depth: u8,
) {
    let width = popup.layout.pixel_width();
    let height = popup.layout.pixel_height();
    popup.renderer.begin(width, height);
    draw_popup(&mut popup.renderer, &popup.menu, &popup.layout, text_style, popup.hovered_item);
    popup.renderer.end();

    conn.put_image(
        ImageFormat::Z_PIXMAP,
        popup.win,
        popup.gc,
        width as u16,
        height as u16,
        0,
        0,
        0,
        depth,
        popup.renderer.data(),
    )
    .unwrap();
}

// Bar rendering moved to `core::bar::render_bar` (imported above) so it is
// shared with the Wayland backend.
