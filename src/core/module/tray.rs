/// System Tray module — implements the StatusNotifierHost side of the
/// freedesktop StatusNotifier specification (the modern replacement for the
/// ancient XEMBED/systray protocol used by KDE Plasma, GNOME extensions,
/// Waybar, Polybar, etc.)
///
/// Architecture
/// ─────────────
/// A Tokio runtime is spawned once on construction.  Inside that runtime
/// the following tasks run:
///
///  1. **Watcher task** — registers the process as a `StatusNotifierWatcher`
///     on the session bus (if no watcher is already present) and also
///     registers ourselves as a `StatusNotifierHost`.  It listens for
///     `RegisterStatusNotifierItem` calls and `StatusNotifierItemRegistered`
///     signals so it always has a fresh list of registered SNI service names.
///
///  2. **Icon-fetch loop** — when `update()` is called from the bar's main
///     thread the module sends the current item list via a channel, fetches
///     icon data for each item concurrently, converts to ARGB32 and stores
///     the result in an `Arc<Mutex<Vec<IconData>>>` shared with `view()`.
///
/// The SNI specification:
///   https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use tokio::sync::watch;
use zbus::{Connection, ConnectionBuilder, interface, proxy};

use super::{IconData, Module, ModuleView};
use crate::core::event::{ClickEvent, MouseButton};
use crate::renderer::primitives::TextStyle;

// ─── DBus proxy for a StatusNotifierItem ────────────────────────────────────

#[proxy(
    interface = "org.kde.StatusNotifierItem",
    default_path = "/StatusNotifierItem"
)]
trait StatusNotifierItem {
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn icon_theme_path(&self) -> zbus::Result<String>;

    /// Returns an array of (width, height, data) structs.
    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;

    /// Show the application or bring its window to the foreground.
    async fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// Show the item's context menu at the given screen coordinates.
    async fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// Middle-click action (alternate activation).
    async fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;
}

// ─── StatusNotifierWatcher interface implementation ──────────────────────────

/// Shared state between the DBus interface object and the rest of the module.
#[derive(Default, Clone)]
struct WatcherState {
    items: Arc<Mutex<Vec<String>>>,
    hosts: Arc<Mutex<Vec<String>>>,
}

struct StatusNotifierWatcher {
    state: WatcherState,
    item_tx: tokio::sync::watch::Sender<Vec<String>>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcher {
    /// Called by SNI clients to register themselves.
    async fn register_status_notifier_item(
        &mut self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .map(|s| s.to_string())
            .unwrap_or_else(|| service.to_string());

        // Normalise: if the caller passed a bare object path, prepend the sender name.
        let full = if service.starts_with('/') {
            format!("{}{}", sender, service)
        } else {
            service.to_string()
        };

        {
            let mut items = self.state.items.lock().unwrap();
            if !items.contains(&full) {
                items.push(full.clone());
                log::info!("[tray] registered SNI item: {}", full);
            }
        }
        let items = self.state.items.lock().unwrap().clone();
        let _ = self.item_tx.send(items);
        Ok(())
    }

    /// Called by StatusNotifierHost implementations.
    async fn register_status_notifier_host(&mut self, service: &str) -> zbus::fdo::Result<()> {
        let mut hosts = self.state.hosts.lock().unwrap();
        if !hosts.contains(&service.to_string()) {
            hosts.push(service.to_string());
        }
        Ok(())
    }

    /// Remove an item when its owner disconnects.
    async fn unregister_status_notifier_item(&mut self, service: &str) -> zbus::fdo::Result<()> {
        {
            let mut items = self.state.items.lock().unwrap();
            items.retain(|s| s != service);
        }
        let items = self.state.items.lock().unwrap().clone();
        let _ = self.item_tx.send(items);
        Ok(())
    }

    /// Properties required by the SNI specification.
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.state.items.lock().unwrap().clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        !self.state.hosts.lock().unwrap().is_empty()
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

// ─── Icon resolution helpers ─────────────────────────────────────────────────

/// Decode SNI pixmap data into ARGB32 bytes.
/// SNI pixmaps are big-endian ARGBs (4 bytes per pixel); we need to convert to
/// native-endian ARGB32 as expected by Cairo.
fn decode_sni_pixmap(width: i32, height: i32, raw: &[u8]) -> Option<IconData> {
    let w = width as u32;
    let h = height as u32;
    let expected = (w * h * 4) as usize;
    if raw.len() < expected {
        return None;
    }
    // SNI sends ARGB big-endian; Cairo wants ARGB native-endian (little-endian on x86).
    let mut pixels = vec![0u8; expected];
    for i in (0..expected).step_by(4) {
        let a = raw[i];
        let r = raw[i + 1];
        let g = raw[i + 2];
        let b = raw[i + 3];
        // Cairo ARGB32 little-endian layout: B G R A
        pixels[i] = b;
        pixels[i + 1] = g;
        pixels[i + 2] = r;
        pixels[i + 3] = a;
    }
    Some(IconData {
        pixels,
        width: w,
        height: h,
    })
}

/// Try to load an icon by name from the XDG icon theme.
/// Falls back to returning `None` if the icon cannot be found.
fn load_icon_by_name(name: &str, theme_path: &str, size: u32) -> Option<IconData> {
    // Build a list of candidate paths.
    let mut search_dirs: Vec<std::path::PathBuf> = Vec::new();
    if !theme_path.is_empty() {
        search_dirs.push(std::path::PathBuf::from(theme_path));
    }
    // Standard XDG icon directories
    if let Some(home) = std::env::var_os("HOME") {
        search_dirs.push(std::path::PathBuf::from(&home).join(".local/share/icons"));
        search_dirs.push(std::path::PathBuf::from(&home).join(".icons"));
    }
    search_dirs.push(std::path::PathBuf::from("/usr/share/icons"));
    search_dirs.push(std::path::PathBuf::from("/usr/share/pixmaps"));

    let sizes = [size, 48, 32, 64, 24, 16, 128, 256];
    let extensions = ["png", "svg", "xpm"];

    for dir in &search_dirs {
        // First try size-specific sub-directories.
        for &sz in &sizes {
            for ext in &extensions {
                // Hicolor-style: <theme>/<size>x<size>/apps/
                let candidates = [
                    dir.join(format!("hicolor/{}x{}/apps/{}.{}", sz, sz, name, ext)),
                    dir.join(format!("hicolor/{}x{}/status/{}.{}", sz, sz, name, ext)),
                    dir.join(format!("{}x{}/apps/{}.{}", sz, sz, name, ext)),
                    dir.join(format!("{}x{}/{}.{}", sz, sz, name, ext)),
                    dir.join(format!("{}.{}", name, ext)),
                ];
                for path in &candidates {
                    if path.exists() {
                        if let Some(data) = load_image_file(path, size) {
                            return Some(data);
                        }
                    }
                }
            }
        }
        // Flat fallback: <dir>/<name>.png
        for ext in &extensions {
            let path = dir.join(format!("{}.{}", name, ext));
            if path.exists() {
                if let Some(data) = load_image_file(&path, size) {
                    return Some(data);
                }
            }
        }
    }
    None
}

/// Load an image file and convert to ARGB32 at the target `size`.
fn load_image_file(path: &std::path::Path, size: u32) -> Option<IconData> {
    let img = image::open(path).ok()?;
    let img = img.resize(size, size, image::imageops::FilterType::Triangle);
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    // Convert RGBA to Cairo ARGB32 (little-endian: B G R A).
    let raw = rgba.into_raw();
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for i in (0..(w * h * 4) as usize).step_by(4) {
        let r = raw[i];
        let g = raw[i + 1];
        let b = raw[i + 2];
        let a = raw[i + 3];
        pixels[i] = b;
        pixels[i + 1] = g;
        pixels[i + 2] = r;
        pixels[i + 3] = a;
    }
    Some(IconData {
        pixels,
        width: w,
        height: h,
    })
}

// ─── Icon fetching ────────────────────────────────────────────────────────────

/// Parse an SNI service string into (bus_name, object_path).
fn parse_sni_service(service: &str) -> (String, String) {
    if let Some(idx) = service.find('/') {
        (service[..idx].to_owned(), service[idx..].to_owned())
    } else {
        (service.to_owned(), "/StatusNotifierItem".to_owned())
    }
}

/// Fetch the best icon for one SNI item.
async fn fetch_icon(conn: &Connection, service: &str, icon_size: u32) -> Option<IconData> {
    let (bus_name, object_path) = parse_sni_service(service);
    let proxy = StatusNotifierItemProxy::builder(conn)
        .destination(bus_name)
        .ok()?
        .path(object_path)
        .ok()?
        .build()
        .await
        .ok()?;

    // Prefer icon_pixmap (has the actual pixels).
    if let Ok(pixmaps) = proxy.icon_pixmap().await {
        // Pick the pixmap closest to the desired size.
        let best = pixmaps.iter().min_by_key(|(w, h, _)| {
            let s = (*w).max(*h) as i64;
            (s - icon_size as i64).abs()
        });
        if let Some((w, h, data)) = best {
            if let Some(icon) = decode_sni_pixmap(*w, *h, data) {
                return Some(icon);
            }
        }
    }

    // Fall back to icon_name + optional theme path.
    let icon_name = proxy.icon_name().await.unwrap_or_default();
    if !icon_name.is_empty() {
        let theme_path = proxy.icon_theme_path().await.unwrap_or_default();
        if let Some(icon) = load_icon_by_name(&icon_name, &theme_path, icon_size) {
            return Some(icon);
        }
    }

    None
}

// ─── TrayModule ───────────────────────────────────────────────────────────────

pub struct TrayModule {
    /// Tokio runtime that owns all async tasks.
    rt: Arc<Runtime>,
    /// Receives the current list of registered SNI item service names.
    item_rx: watch::Receiver<Vec<String>>,
    /// Shared icon cache: service_name → IconData.
    icon_cache: Arc<Mutex<HashMap<String, IconData>>>,
    /// Rendered icons for the current frame (set by `update()`).
    current_icons: Arc<Mutex<Vec<IconData>>>,
    /// Service names in the same order as `current_icons` (for click dispatch).
    current_items: Arc<Mutex<Vec<String>>>,
    /// DBus connection shared with the fetch tasks.
    conn: Arc<Mutex<Option<Connection>>>,
    /// Icon size (pixels).
    icon_size: u32,
}

impl TrayModule {
    pub fn new(icon_size: u32) -> Self {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("beebar-tray")
                .enable_all()
                .build()
                .expect("failed to build tokio runtime for tray"),
        );

        let watcher_state = WatcherState::default();
        let (item_tx, item_rx) = watch::channel(Vec::new());
        let icon_cache: Arc<Mutex<HashMap<String, IconData>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let conn_cell: Arc<Mutex<Option<Connection>>> = Arc::new(Mutex::new(None));

        // Spawn the watcher background task.
        {
            let state_clone = watcher_state.clone();
            let conn_cell_clone = conn_cell.clone();
            let item_tx_clone = item_tx.clone();
            rt.spawn(async move {
                run_watcher(state_clone, conn_cell_clone, item_tx_clone).await;
            });
        }

        Self {
            rt,
            item_rx,
            icon_cache,
            current_icons: Arc::new(Mutex::new(Vec::new())),
            current_items: Arc::new(Mutex::new(Vec::new())),
            conn: conn_cell,
            icon_size,
        }
    }
}

/// Registers the StatusNotifierWatcher and StatusNotifierHost on the session bus,
/// then watches for new items arriving via `item_tx`.
async fn run_watcher(
    state: WatcherState,
    conn_cell: Arc<Mutex<Option<Connection>>>,
    item_tx: watch::Sender<Vec<String>>,
) {
    // Build the DBus connection and register the watcher interface.
    let watcher_obj = StatusNotifierWatcher {
        state: state.clone(),
        item_tx: item_tx.clone(),
    };

    let conn = match ConnectionBuilder::session()
        .and_then(|b| b.name("org.kde.StatusNotifierWatcher"))
        .and_then(|b| b.serve_at("/StatusNotifierWatcher", watcher_obj))
    {
        Ok(builder) => match builder.build().await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("[tray] could not register StatusNotifierWatcher: {e}");
                // Another watcher is already running (e.g. KDE Plasma).
                // Just get a plain session bus connection so we can still
                // listen for newly registered SNI items.
                match Connection::session().await {
                    Ok(c) => c,
                    Err(e2) => {
                        log::error!("[tray] cannot connect to session bus: {e2}");
                        return;
                    }
                }
            }
        },
        Err(e) => {
            log::warn!("[tray] DBus builder error: {e}");
            return;
        }
    };

    // Register ourselves as a StatusNotifierHost so apps know a host is present.
    if let Ok(proxy) = zbus::fdo::DBusProxy::new(&conn).await {
        let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let _ = proxy
            .request_name(
                host_name.as_str().try_into().unwrap(),
                zbus::fdo::RequestNameFlags::DoNotQueue.into(),
            )
            .await;
    }
    {
        // Also call RegisterStatusNotifierHost on the watcher itself if it is
        // hosted by another process (e.g. KDE Plasma).
        if let Ok(watcher_proxy) = zbus::Proxy::new(
            &conn,
            "org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        )
        .await
        {
            let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
            let _ = watcher_proxy
                .call_method("RegisterStatusNotifierHost", &(host_name.as_str(),))
                .await;
        }
    }

    // Store connection for use by fetch tasks.
    {
        let mut guard = conn_cell.lock().unwrap();
        *guard = Some(conn.clone());
    }

    log::info!("[tray] StatusNotifierWatcher/Host registered on session bus");

    // Subscribe to StatusNotifierItemRegistered signal so we discover items
    // that were registered before (or after) we connected.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.kde.StatusNotifierWatcher")
        .and_then(|b| b.member("StatusNotifierItemRegistered"))
        .map(|b| b.build());

    if let Ok(rule) = rule {
        match zbus::MessageStream::for_match_rule(rule, &conn, None).await {
            Ok(stream) => {
                use futures_util::StreamExt;
                let mut stream = Box::pin(stream);
                loop {
                    match stream.next().await {
                        Some(Ok(msg)) => {
                            if let Ok((service,)) = msg.body().deserialize::<(String,)>() {
                                let mut items = state.items.lock().unwrap();
                                if !items.contains(&service) {
                                    items.push(service);
                                }
                                let snapshot = items.clone();
                                drop(items);
                                let _ = item_tx.send(snapshot);
                            }
                        }
                        Some(Err(e)) => {
                            log::warn!("[tray] signal stream error: {e}");
                            break;
                        }
                        None => break,
                    }
                }
            }
            Err(e) => log::warn!("[tray] could not subscribe to SNI signals: {e}"),
        }
    }

    // Keep the connection alive indefinitely.
    std::future::pending::<()>().await;
}

impl Module for TrayModule {
    fn update(&mut self) {
        // Snapshot the current item list.
        let items: Vec<String> = self.item_rx.borrow().clone();

        let icon_cache = self.icon_cache.clone();
        let current_icons = self.current_icons.clone();
        let current_items = self.current_items.clone();
        let conn_cell = self.conn.clone();
        let icon_size = self.icon_size;

        // Fetch icons for any items not yet in cache.
        // This runs on the tokio runtime and completes asynchronously; the
        // results will be visible on the *next* `update()` / `view()` call.
        self.rt.spawn(async move {
            // Get a connection if available.
            let conn = {
                let guard = conn_cell.lock().unwrap();
                guard.clone()
            };
            let conn = match conn {
                Some(c) => c,
                None => return,
            };

            let mut new_icons: Vec<IconData> = Vec::new();
            let mut new_items: Vec<String> = Vec::new();
            for service in &items {
                // Check cache first.
                let cached = {
                    let cache = icon_cache.lock().unwrap();
                    cache.get(service).cloned()
                };
                if let Some(icon) = cached {
                    new_icons.push(icon);
                    new_items.push(service.clone());
                    continue;
                }
                // Fetch from DBus.
                if let Some(icon) = fetch_icon(&conn, service, icon_size).await {
                    {
                        let mut cache = icon_cache.lock().unwrap();
                        cache.insert(service.clone(), icon.clone());
                    }
                    new_icons.push(icon);
                    new_items.push(service.clone());
                }
            }

            // Evict cache entries for items that are no longer registered.
            {
                let mut cache = icon_cache.lock().unwrap();
                cache.retain(|k, _| items.contains(k));
            }

            *current_icons.lock().unwrap() = new_icons;
            *current_items.lock().unwrap() = new_items;
        });
    }

    fn click(&mut self, event: ClickEvent) {
        // Determine which icon slot was hit.
        // `event.x` is relative to the module's left edge (after padding).
        let padding_left = 4.0_f64;
        let icon_size = self.icon_size as f64;
        let spacing = 4.0_f64;
        let slot = icon_size + spacing;

        let rel = (event.x - padding_left).max(0.0);
        let idx = (rel / slot) as usize;

        let service = {
            let items = self.current_items.lock().unwrap();
            items.get(idx).cloned()
        };
        let service = match service {
            Some(s) => s,
            None => return,
        };

        // Clone the connection out of the Mutex before spawning so the future
        // owns both the connection and the service string ('static lifetime).
        let conn = match self.conn.lock().unwrap().clone() {
            Some(c) => c,
            None => return,
        };

        let x = event.x as i32;
        let y = event.y as i32;

        match event.button {
            MouseButton::Left => {
                self.rt.spawn(async move {
                    let (bus, path) = parse_sni_service(&service);
                    let proxy = StatusNotifierItemProxy::builder(&conn)
                        .destination(bus)
                        .ok()
                        .and_then(|b| b.path(path).ok());
                    if let Some(builder) = proxy {
                        match builder.build().await {
                            Ok(p) => {
                                let _ = p.activate(x, y).await;
                            }
                            Err(e) => log::debug!("[tray] proxy build: {e}"),
                        }
                    }
                });
            }
            MouseButton::Right => {
                self.rt.spawn(async move {
                    let (bus, path) = parse_sni_service(&service);
                    let proxy = StatusNotifierItemProxy::builder(&conn)
                        .destination(bus)
                        .ok()
                        .and_then(|b| b.path(path).ok());
                    if let Some(builder) = proxy {
                        match builder.build().await {
                            Ok(p) => {
                                let _ = p.context_menu(x, y).await;
                            }
                            Err(e) => log::debug!("[tray] proxy build: {e}"),
                        }
                    }
                });
            }
            MouseButton::Middle => {
                self.rt.spawn(async move {
                    let (bus, path) = parse_sni_service(&service);
                    let proxy = StatusNotifierItemProxy::builder(&conn)
                        .destination(bus)
                        .ok()
                        .and_then(|b| b.path(path).ok());
                    if let Some(builder) = proxy {
                        match builder.build().await {
                            Ok(p) => {
                                let _ = p.secondary_activate(x, y).await;
                            }
                            Err(e) => log::debug!("[tray] proxy build: {e}"),
                        }
                    }
                });
            }
            MouseButton::Other(_) => {}
        }
    }

    fn view(&self) -> ModuleView {
        let icons = self.current_icons.lock().unwrap().clone();
        ModuleView {
            text: String::new(),
            style: TextStyle::default(),
            background: None,
            padding: (4.0, 4.0),
            icons,
            icon_spacing: 4.0,
        }
    }
}
