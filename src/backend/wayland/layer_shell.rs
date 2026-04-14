use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, LoopSignal,
        },
        calloop_wayland_source::WaylandSource,
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        slot::SlotPool,
        Shm, ShmHandler,
    },
};
use wayland_client::{
    globals::registry_queue_init, protocol::wl_output, Connection, QueueHandle,
};

use crate::core::bar::Bar;
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
    loop_signal: LoopSignal,
}

pub fn run_layer_shell(bar: &mut Bar) {
    let conn = Connection::connect_to_env().expect("Failed to connect to Wayland");
    let (globals, event_queue) =
        registry_queue_init(&conn).expect("Failed to init registry");
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).expect("No wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("No wlr_layer_shell");
    let shm = Shm::bind(&globals, &qh).expect("No wl_shm");

    let surface = compositor.create_surface(&qh);

    let height = bar.height;
    let width = bar.width;

    let layer_surface = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Top,
        Some("beebar"),
        None,
    );
    layer_surface.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    layer_surface.set_size(0, height);
    layer_surface.set_exclusive_zone(height as i32);
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer_surface.commit();

    // Allocate pool large enough for up to 4K-wide bar at the configured height
    let pool = SlotPool::new((3840 * height * 4) as usize, &shm)
        .expect("Failed to create slot pool");

    // Take ownership of the bar
    let bar_owned = std::mem::replace(bar, Bar::new(0, 0));

    let mut event_loop: EventLoop<WaylandState> =
        EventLoop::try_new().expect("Failed to create event loop");
    let loop_handle = event_loop.handle();
    let loop_signal = event_loop.get_signal();

    // Drive Wayland I/O from calloop
    WaylandSource::new(conn, event_queue)
        .insert(loop_handle.clone())
        .expect("Failed to insert Wayland source");

    // Periodic timer: update modules and redraw every 500 ms
    loop_handle
        .insert_source(
            Timer::from_duration(std::time::Duration::from_millis(500)),
            |_, _, state: &mut WaylandState| {
                if state.configured {
                    state.bar.update_all();
                    draw_frame(state);
                }
                TimeoutAction::ToDuration(std::time::Duration::from_millis(500))
            },
        )
       .expect("Failed to insert update timer");

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
        loop_signal,
    };

    event_loop
        .run(None, &mut state, |state| {
            if state.exit {
                state.loop_signal.stop();
            }
        })
        .expect("Event loop error");
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
            state.renderer.measure_text(&view.text, &view.style) + view.padding.0 + view.padding.1
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

            // Center text vertically
            let y = (height as f64 - view.style.font_size) / 2.0;
            state.renderer.draw_text(
                Point {
                    x: region.x + view.padding.0,
                    y,
                },
                &view.text,
                &view.style,
            );
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
    fn closed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
    ) {
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

delegate_compositor!(WaylandState);
delegate_output!(WaylandState);
delegate_layer!(WaylandState);
delegate_shm!(WaylandState);
delegate_registry!(WaylandState);

