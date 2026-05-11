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
use zbus::{
    Connection, ConnectionBuilder, interface, proxy,
    zvariant::{OwnedObjectPath, OwnedValue},
};

use super::{IconData, Module, ModuleChrome, ModuleView};
use crate::core::event::{ClickEvent, MouseButton};
use crate::core::popup::{PopupItemKind, PopupMenu, PopupMenuItem};
use crate::renderer::primitives::TextStyle;

// ─── DBus proxy for a StatusNotifierItem ────────────────────────────────────

type DbusMenuLayout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

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

    #[zbus(property)]
    fn menu(&self) -> zbus::Result<OwnedObjectPath>;

    /// Show the application or bring its window to the foreground.
    async fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// Show the item's context menu at the given screen coordinates.
    async fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// Middle-click action (alternate activation).
    async fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;
}

#[proxy(interface = "com.canonical.dbusmenu")]
trait DbusMenu {
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<&str>,
    ) -> zbus::Result<(u32, DbusMenuLayout)>;

    fn event(&self, id: i32, event_id: &str, data: OwnedValue, timestamp: u32) -> zbus::Result<()>;

    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;
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

        let full = normalize_registered_item(&sender, service);
        insert_registered_item(&self.state, &self.item_tx, full);
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
    async fn unregister_status_notifier_item(
        &mut self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .map(|s| s.to_string())
            .unwrap_or_else(|| service.to_string());
        let full = normalize_registered_item(&sender, service);
        remove_registered_item(&self.state, &self.item_tx, &full);
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
    // Some AppIndicator implementations publish IconName as a real file path
    // instead of a theme icon name. Handle those directly before falling back
    // to theme lookups.
    let normalized_name = name.strip_prefix("file://").unwrap_or(name);
    let icon_path = std::path::Path::new(normalized_name);
    if icon_path.is_absolute() && icon_path.exists() {
        if let Some(data) = load_image_file(icon_path, size) {
            return Some(data);
        }
    }

    if !theme_path.is_empty() {
        let themed_path = std::path::Path::new(theme_path).join(icon_path);
        if themed_path.exists() {
            if let Some(data) = load_image_file(&themed_path, size) {
                return Some(data);
            }
        }
    }

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

    let extensions = ["png", "svg", "xpm"];

    for dir in &search_dirs {
        // Pass 1: fixed-size subdirs we expect to exist. Listed in order of
        // preference around the requested size, including 22 — the size that
        // common app indicators (nm-applet, etc.) install icons under.
        let preferred_sizes: [u32; 9] = [size, 48, 32, 22, 24, 64, 16, 128, 256];
        for &sz in &preferred_sizes {
            for ext in &extensions {
                let candidates = [
                    dir.join(format!("hicolor/{}x{}/apps/{}.{}", sz, sz, name, ext)),
                    dir.join(format!("hicolor/{}x{}/status/{}.{}", sz, sz, name, ext)),
                    dir.join(format!("hicolor/{}x{}/devices/{}.{}", sz, sz, name, ext)),
                    dir.join(format!("{}x{}/apps/{}.{}", sz, sz, name, ext)),
                    dir.join(format!("{}x{}/{}.{}", sz, sz, name, ext)),
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

        // Pass 2: scan whatever <N>x<N> subdirectories actually exist under
        // the current dir's hicolor/ — handles unusual icon-theme sizes that
        // aren't in the preferred list.
        for theme_root in [dir.join("hicolor"), dir.clone()] {
            if let Some(found) = scan_size_subdirs(&theme_root, name, size, &extensions) {
                return Some(found);
            }
        }

        // Pass 3: flat fallback: <dir>/<name>.<ext>
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

/// Walk a theme root looking for `<N>x<N>/(apps|status|devices)/<name>.<ext>`
/// directories. Picks the size closest to `target_size`.
fn scan_size_subdirs(
    theme_root: &std::path::Path,
    name: &str,
    target_size: u32,
    extensions: &[&str],
) -> Option<IconData> {
    let entries = std::fs::read_dir(theme_root).ok()?;
    let mut sized: Vec<(u32, std::path::PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().into_owned();
        // Match "NxN" (16x16, 22x22, 48x48, etc.)
        if let Some((w, h)) = dir_name.split_once('x') {
            if w == h {
                if let Ok(n) = w.parse::<u32>() {
                    sized.push((n, path));
                }
            }
        }
    }
    // Try closest size first.
    sized.sort_by_key(|(n, _)| (*n as i64 - target_size as i64).abs());
    for (_, dir) in sized {
        for sub in ["apps", "status", "devices"] {
            for ext in extensions {
                let candidate = dir.join(sub).join(format!("{}.{}", name, ext));
                if candidate.exists() {
                    if let Some(data) = load_image_file(&candidate, target_size) {
                        return Some(data);
                    }
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

#[derive(Debug, Clone)]
enum InlineMenuItemKind {
    Action,
    Separator,
    Checkbox(bool),
    Submenu(Vec<InlineTrayMenuItem>),
}

#[derive(Debug, Clone)]
struct InlineTrayMenuItem {
    id: i32,
    label: String,
    enabled: bool,
    kind: InlineMenuItemKind,
}

#[derive(Debug, Clone)]
struct InlineTrayMenu {
    service: String,
    anchor_x: f64,
    items: Vec<InlineTrayMenuItem>,
    /// Stack of (parent_label, parent_items) capturing the navigation path.
    /// Empty when at the root menu.
    breadcrumbs: Vec<(String, Vec<InlineTrayMenuItem>)>,
}

/// Parse an SNI service string into (bus_name, object_path).
fn parse_sni_service(service: &str) -> (String, String) {
    if let Some(idx) = service.find('/') {
        (service[..idx].to_owned(), service[idx..].to_owned())
    } else {
        (service.to_owned(), "/StatusNotifierItem".to_owned())
    }
}

fn normalize_registered_item(sender: &str, service: &str) -> String {
    if service.starts_with('/') {
        format!("{}{}", sender, service)
    } else {
        service.to_string()
    }
}

fn mutate_items<F>(state: &WatcherState, item_tx: &watch::Sender<Vec<String>>, mutator: F) -> bool
where
    F: FnOnce(&mut Vec<String>) -> bool,
{
    let snapshot = {
        let mut items = state.items.lock().unwrap();
        if !mutator(&mut items) {
            return false;
        }
        items.clone()
    };
    let _ = item_tx.send(snapshot);
    true
}

fn insert_registered_item(
    state: &WatcherState,
    item_tx: &watch::Sender<Vec<String>>,
    service: String,
) {
    let logged = service.clone();
    if mutate_items(state, item_tx, move |items| {
        if items.contains(&service) {
            false
        } else {
            items.push(service);
            true
        }
    }) {
        log::info!("[tray] registered SNI item: {}", logged);
    }
}

fn matches_registered_item(item: &str, service: &str) -> bool {
    if item == service {
        return true;
    }

    let (item_bus, item_path) = parse_sni_service(item);
    if service.starts_with('/') {
        return item_path == service;
    }

    let (service_bus, service_path) = parse_sni_service(service);
    item_bus == service_bus && item_path == service_path
}

fn remove_registered_item(
    state: &WatcherState,
    item_tx: &watch::Sender<Vec<String>>,
    service: &str,
) {
    mutate_items(state, item_tx, |items| {
        let initial_len = items.len();
        items.retain(|item| !matches_registered_item(item, service));
        initial_len != items.len()
    });
}

fn remove_items_for_bus_name(
    state: &WatcherState,
    item_tx: &watch::Sender<Vec<String>>,
    bus_name: &str,
) {
    mutate_items(state, item_tx, |items| {
        let initial_len = items.len();
        items.retain(|item| parse_sni_service(item).0 != bus_name);
        initial_len != items.len()
    });
}

fn merge_registered_items(
    state: &WatcherState,
    item_tx: &watch::Sender<Vec<String>>,
    services: Vec<String>,
) {
    mutate_items(state, item_tx, move |items| {
        let mut changed = false;
        for service in services {
            if !items.contains(&service) {
                items.push(service);
                changed = true;
            }
        }
        changed
    });
}

fn menu_prop_string(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| String::try_from(value).ok())
}

fn menu_prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    props.get(key).and_then(|value| bool::try_from(value).ok())
}

fn clean_menu_label(label: String) -> String {
    let mut normalized = String::new();
    let mut chars = label.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '_' {
            if matches!(chars.peek(), Some('_')) {
                normalized.push('_');
                chars.next();
            }
            continue;
        }

        if ch == '\t' {
            normalized.push(' ');
        } else {
            normalized.push(ch);
        }
    }

    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_inline_menu_item(value: OwnedValue) -> Option<InlineTrayMenuItem> {
    let (id, props, children): DbusMenuLayout = value.try_into().ok()?;
    if !menu_prop_bool(&props, "visible").unwrap_or(true) {
        return None;
    }

    let item_type = menu_prop_string(&props, "type");
    if matches!(item_type.as_deref(), Some("separator")) {
        return Some(InlineTrayMenuItem {
            id,
            label: String::new(),
            enabled: false,
            kind: InlineMenuItemKind::Separator,
        });
    }

    let label = clean_menu_label(menu_prop_string(&props, "label").unwrap_or_default());
    if label.is_empty() {
        return None;
    }
    let enabled = menu_prop_bool(&props, "enabled").unwrap_or(true);

    // Toggle types: "checkmark" or "radio". Use toggle-state (0/1, -1=unset).
    let toggle_type = menu_prop_string(&props, "toggle-type");
    let toggle_state = props
        .get("toggle-state")
        .and_then(|value| i32::try_from(value).ok());
    if matches!(toggle_type.as_deref(), Some("checkmark") | Some("radio")) {
        return Some(InlineTrayMenuItem {
            id,
            label,
            enabled,
            kind: InlineMenuItemKind::Checkbox(toggle_state.unwrap_or(0) > 0),
        });
    }

    // Submenu: either explicit children-display=submenu, or just non-empty children.
    let children_display = menu_prop_string(&props, "children-display");
    let has_children = !children.is_empty();
    if matches!(children_display.as_deref(), Some("submenu")) || has_children {
        let sub_items = children
            .into_iter()
            .filter_map(parse_inline_menu_item)
            .collect::<Vec<_>>();
        if !sub_items.is_empty() {
            return Some(InlineTrayMenuItem {
                id,
                label,
                enabled,
                kind: InlineMenuItemKind::Submenu(sub_items),
            });
        }
    }

    Some(InlineTrayMenuItem {
        id,
        label,
        enabled,
        kind: InlineMenuItemKind::Action,
    })
}

async fn fetch_inline_menu(
    conn: &Connection,
    service: &str,
    anchor_x: f64,
) -> Option<InlineTrayMenu> {
    let (bus_name, object_path) = parse_sni_service(service);
    let proxy = StatusNotifierItemProxy::builder(conn)
        .destination(bus_name.clone())
        .ok()?
        .path(object_path)
        .ok()?
        .build()
        .await
        .ok()?;

    let menu_path = proxy.menu().await.ok()?;
    let menu_proxy = DbusMenuProxy::builder(conn)
        .destination(bus_name)
        .ok()?
        .path(menu_path)
        .ok()?
        .build()
        .await
        .ok()?;

    let _ = menu_proxy.about_to_show(0).await;
    // Recursion depth -1 = the entire menu tree, so submenus are populated up
    // front and we can navigate without extra round-trips.
    let (_, layout) = menu_proxy
        .get_layout(
            0,
            -1,
            vec![
                "label",
                "enabled",
                "visible",
                "type",
                "toggle-type",
                "toggle-state",
                "children-display",
            ],
        )
        .await
        .ok()?;

    let (_, _, children) = layout;
    let items = children
        .into_iter()
        .filter_map(parse_inline_menu_item)
        .collect::<Vec<_>>();
    if items.is_empty() {
        return None;
    }

    Some(InlineTrayMenu {
        service: service.to_string(),
        anchor_x,
        items,
        breadcrumbs: Vec::new(),
    })
}

async fn trigger_inline_menu_item(conn: &Connection, service: &str, item_id: i32) {
    let (bus_name, object_path) = parse_sni_service(service);
    let proxy = match StatusNotifierItemProxy::builder(conn)
        .destination(bus_name.clone())
        .ok()
        .and_then(|builder| builder.path(object_path).ok())
    {
        Some(builder) => match builder.build().await {
            Ok(proxy) => proxy,
            Err(error) => {
                log::debug!("[tray] failed to build menu proxy: {error}");
                return;
            }
        },
        None => return,
    };

    let menu_path = match proxy.menu().await {
        Ok(path) => path,
        Err(error) => {
            log::debug!("[tray] failed to get menu path: {error}");
            return;
        }
    };

    let menu_proxy = match DbusMenuProxy::builder(conn)
        .destination(bus_name)
        .ok()
        .and_then(|builder| builder.path(menu_path).ok())
    {
        Some(builder) => match builder.build().await {
            Ok(proxy) => proxy,
            Err(error) => {
                log::debug!("[tray] failed to build dbusmenu proxy: {error}");
                return;
            }
        },
        None => return,
    };

    let _ = menu_proxy.about_to_show(item_id).await;
    if let Err(error) = menu_proxy
        .event(item_id, "clicked", OwnedValue::from(0i32), 0)
        .await
    {
        log::debug!("[tray] failed to dispatch dbusmenu click: {error}");
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
    /// Inline menu shown when a tray item exposes dbusmenu instead of ContextMenu.
    inline_menu: Arc<Mutex<Option<InlineTrayMenu>>>,
    /// DBus connection shared with the fetch tasks.
    conn: Arc<Mutex<Option<Connection>>>,
    /// Icon size (pixels).
    icon_size: u32,
    chrome: ModuleChrome,
}

impl TrayModule {
    pub fn new(icon_size: u32, chrome: ModuleChrome) -> Self {
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
            inline_menu: Arc::new(Mutex::new(None)),
            conn: conn_cell,
            icon_size,
            chrome,
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
            if let Ok(items) = watcher_proxy
                .get_property::<Vec<String>>("RegisteredStatusNotifierItems")
                .await
            {
                merge_registered_items(&state, &item_tx, items);
            }
        }
    }

    // Store connection for use by fetch tasks.
    {
        let mut guard = conn_cell.lock().unwrap();
        *guard = Some(conn.clone());
    }

    log::info!("[tray] StatusNotifierWatcher/Host registered on session bus");

    tokio::spawn(watch_registered_signals(
        conn.clone(),
        state.clone(),
        item_tx.clone(),
    ));
    tokio::spawn(watch_unregistered_signals(
        conn.clone(),
        state.clone(),
        item_tx.clone(),
    ));
    tokio::spawn(watch_name_owner_changed_signals(conn, state, item_tx));

    // Keep the connection alive indefinitely.
    std::future::pending::<()>().await;
}

async fn watch_registered_signals(
    conn: Connection,
    state: WatcherState,
    item_tx: watch::Sender<Vec<String>>,
) {
    let rule = match zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.kde.StatusNotifierWatcher")
        .and_then(|b| b.member("StatusNotifierItemRegistered"))
        .map(|b| b.build())
    {
        Ok(rule) => rule,
        Err(e) => {
            log::warn!("[tray] could not build SNI registration rule: {e}");
            return;
        }
    };

    match zbus::MessageStream::for_match_rule(rule, &conn, None).await {
        Ok(stream) => {
            use futures_util::StreamExt;
            let mut stream = Box::pin(stream);
            while let Some(message) = stream.next().await {
                match message {
                    Ok(msg) => {
                        if let Ok((service,)) = msg.body().deserialize::<(String,)>() {
                            insert_registered_item(&state, &item_tx, service);
                        }
                    }
                    Err(e) => {
                        log::warn!("[tray] registered-item stream error: {e}");
                        break;
                    }
                }
            }
        }
        Err(e) => log::warn!("[tray] could not subscribe to item registrations: {e}"),
    }
}

async fn watch_unregistered_signals(
    conn: Connection,
    state: WatcherState,
    item_tx: watch::Sender<Vec<String>>,
) {
    let rule = match zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.kde.StatusNotifierWatcher")
        .and_then(|b| b.member("StatusNotifierItemUnregistered"))
        .map(|b| b.build())
    {
        Ok(rule) => rule,
        Err(e) => {
            log::warn!("[tray] could not build SNI unregistration rule: {e}");
            return;
        }
    };

    match zbus::MessageStream::for_match_rule(rule, &conn, None).await {
        Ok(stream) => {
            use futures_util::StreamExt;
            let mut stream = Box::pin(stream);
            while let Some(message) = stream.next().await {
                match message {
                    Ok(msg) => {
                        if let Ok((service,)) = msg.body().deserialize::<(String,)>() {
                            remove_registered_item(&state, &item_tx, &service);
                        }
                    }
                    Err(e) => {
                        log::warn!("[tray] unregistered-item stream error: {e}");
                        break;
                    }
                }
            }
        }
        Err(e) => log::warn!("[tray] could not subscribe to item removals: {e}"),
    }
}

async fn watch_name_owner_changed_signals(
    conn: Connection,
    state: WatcherState,
    item_tx: watch::Sender<Vec<String>>,
) {
    let rule = match zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus")
        .and_then(|b| b.member("NameOwnerChanged"))
        .map(|b| b.build())
    {
        Ok(rule) => rule,
        Err(e) => {
            log::warn!("[tray] could not build NameOwnerChanged rule: {e}");
            return;
        }
    };

    match zbus::MessageStream::for_match_rule(rule, &conn, None).await {
        Ok(stream) => {
            use futures_util::StreamExt;
            let mut stream = Box::pin(stream);
            while let Some(message) = stream.next().await {
                match message {
                    Ok(msg) => {
                        if let Ok((name, old_owner, new_owner)) =
                            msg.body().deserialize::<(String, String, String)>()
                        {
                            if !old_owner.is_empty() && new_owner.is_empty() {
                                remove_items_for_bus_name(&state, &item_tx, &name);
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("[tray] NameOwnerChanged stream error: {e}");
                        break;
                    }
                }
            }
        }
        Err(e) => log::warn!("[tray] could not subscribe to NameOwnerChanged: {e}"),
    }
}

impl Module for TrayModule {
    fn update(&mut self) {
        // Snapshot the current item list.
        let items: Vec<String> = self.item_rx.borrow().clone();

        {
            let mut inline_menu = self.inline_menu.lock().unwrap();
            if inline_menu
                .as_ref()
                .is_some_and(|menu| !items.contains(&menu.service))
            {
                inline_menu.take();
            }
        }

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
        let padding_left = self.chrome.padding.0;
        let icon_size = self.icon_size as f64;
        let spacing = self.chrome.icon_spacing.unwrap_or(4.0);
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

        let x = event.screen_x as i32;
        let y = event.screen_y as i32;

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
                let inline_menu = self.inline_menu.clone();
                let anchor_x = event.bar_x;
                let menu = self.rt.block_on(async move {
                    let (bus, path) = parse_sni_service(&service);
                    let proxy = StatusNotifierItemProxy::builder(&conn)
                        .destination(bus)
                        .ok()
                        .and_then(|b| b.path(path).ok());
                    if let Some(builder) = proxy {
                        match builder.build().await {
                            Ok(p) => {
                                if let Err(error) = p.context_menu(x, y).await {
                                    log::debug!("[tray] context_menu unavailable, falling back to dbusmenu: {error}");
                                    return fetch_inline_menu(&conn, &service, anchor_x).await;
                                }
                            }
                            Err(e) => log::debug!("[tray] proxy build: {e}"),
                        }
                    }
                    None
                });
                *inline_menu.lock().unwrap() = menu;
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
            // Tray items have no meaningful response to wheel events; ignore.
            MouseButton::ScrollUp | MouseButton::ScrollDown | MouseButton::Other(_) => {}
        }
    }

    fn view(&self) -> ModuleView {
        let icons = self.current_icons.lock().unwrap().clone();
        self.chrome.apply(ModuleView {
            text: String::new(),
            text_segments: Vec::new(),
            style: TextStyle::default(),
            background: None,
            icons,
            icon_size: Some(self.icon_size),
            ..Default::default()
        })
    }

    fn popup(&self) -> Option<PopupMenu> {
        let menu = self.inline_menu.lock().unwrap().clone()?;
        let mut items: Vec<PopupMenuItem> = Vec::new();

        // When inside a submenu, prepend a "← parent" back-navigation row.
        if let Some((parent_label, _)) = menu.breadcrumbs.last() {
            items.push(PopupMenuItem {
                label: format!("\u{2039} {}", parent_label),
                enabled: true,
                kind: PopupItemKind::Action,
            });
            // Separator between back row and submenu contents.
            items.push(PopupMenuItem {
                label: String::new(),
                enabled: false,
                kind: PopupItemKind::Separator,
            });
        }

        items.extend(menu.items.iter().map(|item| PopupMenuItem {
            label: item.label.clone(),
            enabled: item.enabled,
            kind: match &item.kind {
                InlineMenuItemKind::Action => PopupItemKind::Action,
                InlineMenuItemKind::Separator => PopupItemKind::Separator,
                InlineMenuItemKind::Checkbox(checked) => PopupItemKind::Checkbox(*checked),
                InlineMenuItemKind::Submenu(_) => PopupItemKind::Submenu,
            },
        }));

        Some(PopupMenu {
            anchor_x: menu.anchor_x,
            items,
        })
    }

    fn popup_click(&mut self, item_index: usize, button: MouseButton) {
        if button != MouseButton::Left {
            return;
        }

        // Borrow the menu mutably so we can either replace it (drill-in) or
        // take it (action/checkbox triggers a real DBus event and dismisses).
        let mut menu_guard = self.inline_menu.lock().unwrap();
        let Some(menu) = menu_guard.as_mut() else {
            return;
        };

        // When breadcrumbs exist the popup prepends a back-nav row (index 0)
        // and a separator (index 1) before the real items.
        let has_back = !menu.breadcrumbs.is_empty();
        let header_rows: usize = if has_back { 2 } else { 0 };

        // Back-nav row clicked → pop the breadcrumb stack.
        if has_back && item_index == 0 {
            if let Some((_, parent_items)) = menu.breadcrumbs.pop() {
                menu.items = parent_items;
            }
            return;
        }

        // Separator row (index 1) is not actionable.
        if has_back && item_index == 1 {
            return;
        }

        let real_index = item_index - header_rows;
        let Some(item) = menu.items.get(real_index).cloned() else {
            return;
        };
        if !item.enabled || matches!(item.kind, InlineMenuItemKind::Separator) {
            return;
        }

        match item.kind {
            InlineMenuItemKind::Submenu(children) => {
                // Drill in: push current items onto the breadcrumb stack and
                // swap to the children. Keep the popup open.
                let parent_items = std::mem::take(&mut menu.items);
                menu.breadcrumbs.push((item.label.clone(), parent_items));
                menu.items = children;
            }
            InlineMenuItemKind::Action | InlineMenuItemKind::Checkbox(_) => {
                let service = menu.service.clone();
                let item_id = item.id;
                drop(menu_guard);
                self.inline_menu.lock().unwrap().take();
                let conn = match self.conn.lock().unwrap().clone() {
                    Some(c) => c,
                    None => return,
                };
                self.rt.spawn(async move {
                    trigger_inline_menu_item(&conn, &service, item_id).await;
                });
            }
            InlineMenuItemKind::Separator => {}
        }
    }

    fn dismiss_popup(&mut self) {
        self.inline_menu.lock().unwrap().take();
    }
}
