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
use crate::core::module::ModuleId;
use crate::core::popup::{POPUP_GAP, PopupLayout, PopupMenu, draw_popup, layout_popup};
use crate::renderer::cairo_renderer::CairoRenderer;
use crate::renderer::primitives::{Point, Rect, Renderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointerFocus {
    None,
    Bar,
    Popup,
    Catcher,
}

struct PopupSurfaceState {
    layer_surface: LayerSurface,
    pool: SlotPool,
    renderer: CairoRenderer,
    owner: ModuleId,
    menu: PopupMenu,
    layout: PopupLayout,
    configured: bool,
    hovered_item: Option<usize>,
}

/// Fullscreen transparent layer surface placed underneath the popup.
/// Captures pointer events anywhere outside the popup so we can dismiss
/// the menu when the user clicks away.
struct PopupCatcherState {
    layer_surface: LayerSurface,
    pool: SlotPool,
    width: u32,
    height: u32,
    configured: bool,
}

struct WaylandState {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    queue_handle: QueueHandle<WaylandState>,
    shm: Shm,
    pool: SlotPool,
    layer_surface: LayerSurface,
    popup: Option<PopupSurfaceState>,
    popup_catcher: Option<PopupCatcherState>,
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
    pointer_focus: PointerFocus,
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
    let bottom = bar.bottom;

    let layer_surface =
        layer_shell.create_layer_surface(&qh, surface, Layer::Top, Some("beebar"), None);
    if bottom {
        layer_surface.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    } else {
        layer_surface.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    }
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
        compositor,
        layer_shell,
        queue_handle: qh.clone(),
        shm,
        pool,
        layer_surface,
        popup: None,
        popup_catcher: None,
        bar: bar_owned,
        renderer: CairoRenderer::new(width, height),
        configured: false,
        width,
        height,
        exit: false,
        pointer: None,
        pointer_pos: (0.0, 0.0),
        pointer_focus: PointerFocus::None,
    };

    // Do a full roundtrip so the compositor processes our layer_surface.commit
    // and sends back the configure event before we enter the main loop.
    event_queue
        .roundtrip(&mut state)
        .expect("initial roundtrip failed");

    // Poll-based event loop: poll the Wayland fd with a short timeout so
    // workspace updates feel immediate without busy-spinning the renderer.
    let update_interval = std::time::Duration::from_secs(1);
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
            sync_popup_surface(&mut state);
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
    state.renderer.draw_rect(
        Rect {
            x: 0.0,
            y: 0.0,
            width: width as f64,
            height: height as f64,
        },
        state.bar.background,
    );

    // Compute layout
    let layout = &state.bar.layout;

    let measure = |id: &String| -> f64 {
        if let Some(view) = state.bar.module_view(id) {
            if !view.icons.is_empty() {
                let icon_size = view.icon_size.unwrap_or_else(|| height.saturating_sub(4)) as f64;
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
        if let Some(view) = state.bar.module_view(&region.id) {
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
                let icon_size = view.icon_size.unwrap_or_else(|| height.saturating_sub(4));
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
                let y = (height as f64 - view.text_height(&state.renderer)) / 2.0
                    + state.bar.text_y_offset;
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

fn sync_popup_surface(state: &mut WaylandState) {
    let active_popup = state.bar.active_popup().and_then(|(owner, menu)| {
        layout_popup(
            &menu,
            state.width as f64,
            &state.bar.text_style,
            &state.renderer,
        )
        .map(|layout| (owner, menu, layout))
    });

    match active_popup {
        Some((owner, menu, layout)) => {
            let popup_width = layout.pixel_width();
            let popup_height = layout.pixel_height();
            let left_margin = layout.x.round() as i32;
            // The popup is on Layer::Overlay with exclusive_zone(0), so the
            // compositor already places it past the bar's exclusive zone. We
            // only add the small visual gap here — adding bar.height again
            // would double the offset.
            let edge_margin = POPUP_GAP.round() as i32;

            // Create the click-catcher first so the popup ends up z-above it
            // (creation order within the same layer determines stacking).
            if state.popup_catcher.is_none() {
                create_popup_catcher(state);
            }

            if state.popup.is_none() {
                let surface = state.compositor.create_surface(&state.queue_handle);
                let layer_surface = state.layer_shell.create_layer_surface(
                    &state.queue_handle,
                    surface,
                    Layer::Overlay,
                    Some("beebar-popup"),
                    None,
                );
                if state.bar.bottom {
                    layer_surface.set_anchor(Anchor::BOTTOM | Anchor::LEFT);
                } else {
                    layer_surface.set_anchor(Anchor::TOP | Anchor::LEFT);
                }
                layer_surface.set_exclusive_zone(0);
                layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);

                let pool = SlotPool::new(
                    (popup_width.max(1) * popup_height.max(1) * 4) as usize,
                    &state.shm,
                )
                .expect("Failed to create Wayland popup slot pool");

                state.popup = Some(PopupSurfaceState {
                    layer_surface,
                    pool,
                    renderer: CairoRenderer::new(popup_width, popup_height),
                    owner: owner.clone(),
                    menu: menu.clone(),
                    layout: layout.clone(),
                    configured: false,
                    hovered_item: None,
                });
            }

            let popup = state.popup.as_mut().unwrap();
            popup.owner = owner;
            popup.menu = menu;
            popup.layout = layout;
            popup.layer_surface.set_size(popup_width, popup_height);
            if state.bar.bottom {
                popup
                    .layer_surface
                    .set_margin(0, 0, edge_margin, left_margin);
            } else {
                popup
                    .layer_surface
                    .set_margin(edge_margin, 0, 0, left_margin);
            }

            if popup.configured {
                draw_popup_frame(popup, &state.bar.text_style);
            } else {
                popup.layer_surface.wl_surface().commit();
            }
        }
        None => {
            state.popup = None;
            state.popup_catcher = None;
            if matches!(
                state.pointer_focus,
                PointerFocus::Popup | PointerFocus::Catcher
            ) {
                state.pointer_focus = PointerFocus::None;
            }
        }
    }
}

fn create_popup_catcher(state: &mut WaylandState) {
    let surface = state.compositor.create_surface(&state.queue_handle);
    let layer_surface = state.layer_shell.create_layer_surface(
        &state.queue_handle,
        surface,
        // Top layer (one below Overlay where the popup lives), so the popup
        // is guaranteed to be above the catcher regardless of compositor
        // creation-order semantics.
        Layer::Top,
        Some("beebar-popup-catcher"),
        None,
    );
    layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer_surface.set_size(0, 0);
    // Ignore other surfaces' exclusive zones so we cover the whole output,
    // including the area reserved by the bar.
    layer_surface.set_exclusive_zone(-1);
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer_surface.commit();

    let pool = SlotPool::new(64 * 64 * 4, &state.shm)
        .expect("Failed to create Wayland popup catcher slot pool");

    state.popup_catcher = Some(PopupCatcherState {
        layer_surface,
        pool,
        width: 0,
        height: 0,
        configured: false,
    });
}

fn draw_popup_catcher_frame(catcher: &mut PopupCatcherState) {
    let width = catcher.width.max(1);
    let height = catcher.height.max(1);
    let stride = width * 4;
    let buf_size = (stride * height) as usize;

    catcher
        .pool
        .resize(buf_size)
        .expect("Failed to resize Wayland popup catcher slot pool");

    let (buffer, canvas) = catcher
        .pool
        .create_buffer(
            width as i32,
            height as i32,
            stride as i32,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        )
        .expect("Failed to create Wayland popup catcher buffer");

    // Fully transparent.
    let copy_len = buf_size.min(canvas.len());
    for byte in &mut canvas[..copy_len] {
        *byte = 0;
    }

    catcher
        .layer_surface
        .wl_surface()
        .attach(Some(buffer.wl_buffer()), 0, 0);
    catcher
        .layer_surface
        .wl_surface()
        .damage_buffer(0, 0, width as i32, height as i32);
    catcher.layer_surface.wl_surface().commit();
}

fn draw_popup_frame(
    popup: &mut PopupSurfaceState,
    text_style: &crate::renderer::primitives::TextStyle,
) {
    let width = popup.layout.pixel_width();
    let height = popup.layout.pixel_height();
    let stride = width * 4;
    let buf_size = (stride * height) as usize;

    popup
        .pool
        .resize(buf_size)
        .expect("Failed to resize Wayland popup slot pool");

    let (buffer, canvas) = popup
        .pool
        .create_buffer(
            width as i32,
            height as i32,
            stride as i32,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        )
        .expect("Failed to create Wayland popup buffer");

    popup.renderer.begin(width, height);
    draw_popup(&mut popup.renderer, &popup.menu, &popup.layout, text_style, popup.hovered_item);
    popup.renderer.end();

    let data = popup.renderer.data();
    let copy_len = buf_size.min(data.len()).min(canvas.len());
    canvas[..copy_len].copy_from_slice(&data[..copy_len]);

    popup
        .layer_surface
        .wl_surface()
        .attach(Some(buffer.wl_buffer()), 0, 0);
    popup
        .layer_surface
        .wl_surface()
        .damage_buffer(0, 0, width as i32, height as i32);
    popup.layer_surface.wl_surface().commit();
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
        if *_layer == self.layer_surface {
            self.exit = true;
        } else if self
            .popup
            .as_ref()
            .is_some_and(|popup| popup.layer_surface == *_layer)
        {
            self.bar.dismiss_all_popups();
            self.popup = None;
            self.popup_catcher = None;
            self.pointer_focus = PointerFocus::None;
        } else if self
            .popup_catcher
            .as_ref()
            .is_some_and(|catcher| catcher.layer_surface == *_layer)
        {
            self.bar.dismiss_all_popups();
            self.popup = None;
            self.popup_catcher = None;
            self.pointer_focus = PointerFocus::None;
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if *_layer == self.layer_surface {
            log::debug!("LayerSurface configure: new_size={:?}", configure.new_size);
            if configure.new_size.0 > 0 {
                self.width = configure.new_size.0;
            }
            if configure.new_size.1 > 0 {
                self.height = configure.new_size.1;
            }
            self.bar.width = self.width;
            self.configured = true;
            self.bar.update_all();
            sync_popup_surface(self);
            draw_frame(self);
        } else if let Some(popup) = self
            .popup
            .as_mut()
            .filter(|popup| popup.layer_surface == *_layer)
        {
            popup.configured = true;
            draw_popup_frame(popup, &self.bar.text_style);
        } else if let Some(catcher) = self
            .popup_catcher
            .as_mut()
            .filter(|catcher| catcher.layer_surface == *_layer)
        {
            if configure.new_size.0 > 0 {
                catcher.width = configure.new_size.0;
            }
            if configure.new_size.1 > 0 {
                catcher.height = configure.new_size.1;
            }
            catcher.configured = true;
            draw_popup_catcher_frame(catcher);
        }
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
        conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_pos = (surface_x, surface_y);
                state.pointer_focus = if surface == *state.layer_surface.wl_surface() {
                    PointerFocus::Bar
                } else if state
                    .popup
                    .as_ref()
                    .is_some_and(|popup| surface == *popup.layer_surface.wl_surface())
                {
                    PointerFocus::Popup
                } else if state
                    .popup_catcher
                    .as_ref()
                    .is_some_and(|catcher| surface == *catcher.layer_surface.wl_surface())
                {
                    PointerFocus::Catcher
                } else {
                    PointerFocus::None
                };
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_pos = (surface_x, surface_y);
                // Update hover highlight when pointer moves over the popup.
                if state.pointer_focus == PointerFocus::Popup {
                    let hover_changed = if let Some(popup) = state.popup.as_mut() {
                        let new_hover = popup
                            .layout
                            .hit_test(&popup.menu.items, surface_x, surface_y);
                        let changed = new_hover != popup.hovered_item;
                        popup.hovered_item = new_hover;
                        changed
                    } else {
                        false
                    };
                    if hover_changed {
                        let style = state.bar.text_style.clone();
                        if let Some(popup) = state.popup.as_mut().filter(|p| p.configured) {
                            draw_popup_frame(popup, &style);
                            conn.flush().ok();
                        }
                    }
                }
            }
            wl_pointer::Event::Leave { .. } => {
                // Clear hover highlight when pointer leaves the popup.
                if state.pointer_focus == PointerFocus::Popup {
                    let style = state.bar.text_style.clone();
                    if let Some(popup) = state.popup.as_mut().filter(|p| p.configured) {
                        popup.hovered_item = None;
                        draw_popup_frame(popup, &style);
                        conn.flush().ok();
                    }
                }
                state.pointer_focus = PointerFocus::None;
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

                if state.pointer_focus == PointerFocus::Popup {
                    if let Some((owner, item_idx)) = state.popup.as_ref().and_then(|popup| {
                        popup
                            .layout
                            .hit_test(&popup.menu.items, px, py)
                            .map(|item_idx| (popup.owner.clone(), item_idx))
                    }) {
                        state.bar.handle_popup_click(&owner, item_idx, mb);
                    } else {
                        state.bar.dismiss_all_popups();
                    }
                    state.bar.update_all();
                    sync_popup_surface(state);
                    draw_frame(state);
                    conn.flush().ok();
                    return;
                }

                if state.pointer_focus == PointerFocus::Catcher {
                    state.bar.dismiss_all_popups();
                    state.bar.update_all();
                    sync_popup_surface(state);
                    draw_frame(state);
                    conn.flush().ok();
                    return;
                }

                if state.popup.is_some() {
                    state.bar.dismiss_all_popups();
                }

                let click = ClickEvent {
                    x: px,
                    bar_x: px,
                    screen_x: px,
                    module_width: 0.0,
                    y: py,
                    bar_y: py,
                    screen_y: py,
                    button: mb,
                };

                // Compute layout regions using immutable borrows of bar + renderer.
                let width = state.width;
                let height = state.height;
                let regions = {
                    let measure = |id: &String| -> f64 {
                        if let Some(view) = state.bar.module_view(id) {
                            if !view.icons.is_empty() {
                                let icon_size_px =
                                    view.icon_size.unwrap_or_else(|| height.saturating_sub(4))
                                        as f64;
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
                state.bar.update_all();
                sync_popup_surface(state);
                draw_frame(state);
                conn.flush().ok();
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
