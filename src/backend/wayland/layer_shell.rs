use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::os::unix::io::AsRawFd;
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat},
};

use crate::core::bar::Bar;
use crate::core::event::{ClickEvent, MouseButton};
use crate::renderer::cairo_renderer::CairoRenderer;
use crate::renderer::color::Color;
use crate::renderer::primitives::{Point, Rect, Renderer};

struct WaylandState {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer_surface: LayerSurface,
    bar: Bar,
    renderer: CairoRenderer,
    configured: bool,
    width: u32,
    height: u32,
    exit: bool,
    /// Raw Wayland pointer object (kept alive to receive events).
    pointer: Option<wl_pointer::WlPointer>,
    /// Last known pointer position on our bar surface.
    pointer_pos: (f64, f64),
}

pub fn run_layer_shell(bar: &mut Bar) {
    let conn = Connection::connect_to_env().expect("Failed to connect to Wayland");
    let (globals, mut event_queue) = registry_queue_init(&conn).expect("Failed to init registry");
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).expect("No wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("No wlr_layer_shell");
    let shm = Shm::bind(&globals, &qh).expect("No wl_shm");

    let surface = compositor.create_surface(&qh);

    let height = bar.height;
    let width = bar.width;

    let layer_surface =
        layer_shell.create_layer_surface(&qh, surface, Layer::Top, Some("beebar"), None);
    layer_surface.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    layer_surface.set_size(0, height);
    layer_surface.set_exclusive_zone(height as i32);
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer_surface.commit();

    // Allocate pool large enough for up to 4K-wide bar at the configured height
    let pool =
        SlotPool::new((3840 * height * 4) as usize, &shm).expect("Failed to create slot pool");

    // Bind wl_seat so pointer events can be received.
    // Fail gracefully — a seat is not strictly required for rendering.
    let _seat: Option<wl_seat::WlSeat> = globals
        .bind::<wl_seat::WlSeat, _, _>(&qh, 1..=8, ())
        .map_err(|e| log::warn!("[wayland] no wl_seat: {e}"))
        .ok();

    // Take ownership of the bar
    let bar_owned = std::mem::replace(bar, Bar::new(0, 0));

    // Grab the raw Wayland socket fd for poll() — EventQueue implements AsFd.
    let wayland_fd = {
        use std::os::unix::io::AsFd;
        event_queue.as_fd().as_raw_fd()
    };

    // Flush all queued requests (create_surface, set_anchor, commit, etc.)
    // so the compositor receives them and can send back a configure event.
    conn.flush()
        .expect("Failed to flush initial Wayland requests");

    let mut state = WaylandState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer_surface,
        bar: bar_owned,
        renderer: CairoRenderer::new(width, height),
        configured: false,
        width,
        height,
        exit: false,
        pointer: None,
        pointer_pos: (0.0, 0.0),
    };

    // Do a full roundtrip so the compositor processes our layer_surface.commit
    // and sends back the configure event before we enter the main loop.
    event_queue
        .roundtrip(&mut state)
        .expect("initial roundtrip failed");

    // Poll-based event loop: poll the Wayland fd with a short timeout so
    // workspace updates feel immediate without busy-spinning the renderer.
    let update_interval = std::time::Duration::from_millis(100);
    let mut next_update = std::time::Instant::now();

    loop {
        conn.flush().expect("flush");

        // How long until the next scheduled module update?
        let now = std::time::Instant::now();
        let timeout_ms: i32 = if now >= next_update {
            0
        } else {
            (next_update - now).as_millis().min(i32::MAX as u128) as i32
        };

        // Prepare to read events (must be done before polling the fd).
        let guard = event_queue.prepare_read();

        // Poll the Wayland socket fd.
        let mut pollfd = libc::pollfd {
            fd: wayland_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pollfd array is valid for the duration of this call.
        let poll_ret = unsafe { libc::poll(&mut pollfd as *mut libc::pollfd, 1, timeout_ms) };

        if poll_ret > 0 && pollfd.revents & libc::POLLIN != 0 {
            // Data available: read it into the event queue.
            if let Some(g) = guard {
                match g.read() {
                    Ok(_) => {}
                    // EAGAIN: data disappeared between poll and read — safe to ignore.
                    Err(wayland_client::backend::WaylandError::Io(e))
                        if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => panic!("failed to read Wayland events: {e}"),
                }
            }
        } else {
            // Timeout or no data; drop the guard without reading.
            drop(guard);
        }

        // Dispatch all events that are now in the queue.
        event_queue
            .dispatch_pending(&mut state)
            .expect("dispatch failed");

        // Periodic module update + redraw.
        let now = std::time::Instant::now();
        if state.configured && now >= next_update {
            next_update = now + update_interval;
            state.bar.update_all();
            draw_frame(&mut state);
            conn.flush().expect("flush after draw");
        }

        if state.exit {
            break;
        }
    }
}

fn draw_frame(state: &mut WaylandState) {
    let width = state.width;
    let height = state.height;
    let stride = width * 4;
    let buf_size = (stride * height) as usize;

    // Ensure the shared-memory pool is large enough for the current dimensions
    state.pool.resize(buf_size).expect("Failed to resize pool");

    let (buffer, canvas) = state
        .pool
        .create_buffer(
            width as i32,
            height as i32,
            stride as i32,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        )
        .expect("Failed to create buffer");

    // Render via cairo
    state.renderer.begin(width, height);

    // Background
    let bg_color = Color::from_hex("#1e1e2e").unwrap_or(Color::BLACK);
    state.renderer.draw_rect(
        Rect {
            x: 0.0,
            y: 0.0,
            width: width as f64,
            height: height as f64,
        },
        bg_color,
    );

    // Compute layout
    let layout = &state.bar.layout;
    let modules = &state.bar.modules;

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
                view.text_width(&state.renderer) + view.padding.0 + view.padding.1
            }
        } else {
            0.0
        }
    };

    let regions = layout.compute(width as f64, &measure);

    // Draw modules
    for region in &regions {
        if let Some(module) = modules.get(&region.id) {
            let view = module.view();

            if let Some(bg) = view.background {
                state.renderer.draw_rect(
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
                // Render tray icons side by side.
                let icon_size = height.saturating_sub(4);
                let mut ix = region.x + view.padding.0;
                let iy = ((height as f64 - icon_size as f64) / 2.0).max(0.0);
                for icon_data in &view.icons {
                    state.renderer.draw_icon(
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
                    state
                        .renderer
                        .draw_text(Point { x, y }, &view.text, &view.style);
                } else {
                    for segment in &view.text_segments {
                        x +=
                            state
                                .renderer
                                .draw_text(Point { x, y }, &segment.text, &segment.style);
                    }
                }
            }
        }
    }

    state.renderer.end();

    // Copy rendered pixels to Wayland buffer
    let data = state.renderer.data();
    let copy_len = buf_size.min(data.len()).min(canvas.len());
    canvas[..copy_len].copy_from_slice(&data[..copy_len]);

    state
        .layer_surface
        .wl_surface()
        .attach(Some(buffer.wl_buffer()), 0, 0);
    state
        .layer_surface
        .wl_surface()
        .damage_buffer(0, 0, width as i32, height as i32);
    state.layer_surface.wl_surface().commit();
}

// --- Smithay handler impls ---

impl CompositorHandler for WaylandState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
        _new_transform: wayland_client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for WaylandState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for WaylandState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        log::debug!("LayerSurface configure: new_size={:?}", configure.new_size);
        if configure.new_size.0 > 0 {
            self.width = configure.new_size.0;
        }
        if configure.new_size.1 > 0 {
            self.height = configure.new_size.1;
        }
        self.configured = true;
        // Draw immediately on initial configure / resize
        self.bar.update_all();
        draw_frame(self);
    }
}

impl ShmHandler for WaylandState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for WaylandState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

// ─── wl_seat: discover pointer capability ────────────────────────────────────

impl Dispatch<wl_seat::WlSeat, ()> for WaylandState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            // Pointer capability bit = 1 (per Wayland protocol spec).
            let has_pointer = match capabilities {
                WEnum::Value(c) => (u32::from(c) & 1) != 0,
                WEnum::Unknown(n) => (n & 1) != 0,
            };
            if has_pointer && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
                log::debug!("[wayland] pointer acquired");
            }
        }
    }
}

// ─── wl_pointer: track position + dispatch click events ──────────────────────

impl Dispatch<wl_pointer::WlPointer, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_pos = (surface_x, surface_y);
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_pos = (surface_x, surface_y);
            }
            wl_pointer::Event::Button {
                button,
                state: btn_state,
                ..
            } => {
                if !matches!(btn_state, WEnum::Value(wl_pointer::ButtonState::Pressed)) {
                    return;
                }
                if !state.configured {
                    return;
                }

                let mb = match button {
                    0x110 => MouseButton::Left,   // BTN_LEFT
                    0x111 => MouseButton::Right,  // BTN_RIGHT
                    0x112 => MouseButton::Middle, // BTN_MIDDLE
                    n => MouseButton::Other(n),
                };
                let (px, py) = state.pointer_pos;
                let click = ClickEvent {
                    x: px,
                    module_width: 0.0,
                    y: py,
                    button: mb,
                };

                // Compute layout regions using immutable borrows of bar + renderer.
                let width = state.width;
                let height = state.height;
                let icon_size_px = height.saturating_sub(4) as f64;
                let regions = {
                    let modules = &state.bar.modules;
                    let measure = |id: &String| -> f64 {
                        if let Some(m) = modules.get(id) {
                            let view = m.view();
                            if !view.icons.is_empty() {
                                let n = view.icons.len() as f64;
                                view.padding.0
                                    + view.padding.1
                                    + n * icon_size_px
                                    + (n - 1.0).max(0.0) * view.icon_spacing
                            } else {
                                view.text_width(&state.renderer) + view.padding.0 + view.padding.1
                            }
                        } else {
                            0.0
                        }
                    };
                    state.bar.layout.compute(width as f64, &measure)
                };
                state.bar.handle_click(&regions, &click);
            }
            _ => {}
        }
    }
}

delegate_compositor!(WaylandState);
delegate_output!(WaylandState);
delegate_layer!(WaylandState);
delegate_shm!(WaylandState);
delegate_registry!(WaylandState);
