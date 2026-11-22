//! Quick-settings panel.
//!
//! Shows an icon in the bar. Clicking it opens a dropdown panel with a row of
//! buttons, sliders and a grid of toggle tiles (see `core::panel` for the
//! drawing). Everything in the panel is defined in the config as shell
//! commands, so it works with whatever tools the system has:
//!
//! - buttons run a command and close the panel (`confirm = true` asks for a
//!   second click first, for things like power off),
//! - sliders read their value from a `get` command (0-100) and write it with
//!   a `set` command,
//! - tiles run a command on click and light up while their `state` command
//!   exits with 0.
//!
//! A slider or tile with `page = "..."` gets an arrow that opens a built-in
//! sub-page in the same panel (sound output/input mixer, Wi-Fi networks,
//! Bluetooth devices, see `pages`).
//!
//! The commands that read state run on a worker thread, once when beebar
//! starts and then every second while the panel is open, so a slow command
//! never blocks drawing.

mod media;
mod pages;

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{IconData, Module, ModuleChrome, ModuleView};
use crate::core::config::{
    ModuleConfig, PanelButtonConfig, PanelSliderConfig, PanelTileConfig, resolve_color,
};
use crate::core::event::{ClickEvent, MouseButton};
use crate::core::panel::{
    PageRow, PanelButton, PanelMedia, PanelSlider, PanelStyle, PanelTarget, PanelTile, PanelView,
};
use crate::core::popup::{HitTarget, PanelPopup, Popup, PopupHit};
use crate::renderer::color::Color;
use crate::renderer::primitives::TextStyle;
use media::MediaState;
use pages::{BuiltPage, PageKind, VolumeTarget};

/// nf-md-cog
const DEFAULT_ICON: &str = "\u{f0493}";
const DEFAULT_WIDTH: f64 = 360.0;
const MIN_WIDTH: f64 = 240.0;
const CORNER_RADIUS: f64 = 22.0;

const BACKGROUND: Color = Color {
    r: 0.118,
    g: 0.118,
    b: 0.180,
    a: 0.96,
};
const BORDER: Color = Color::rgb(0.271, 0.278, 0.353);
const SURFACE: Color = Color::rgb(0.192, 0.196, 0.267);
const ACCENT: Color = Color::rgb(0.796, 0.651, 0.969);
const ACCENT_TEXT: Color = Color::rgb(0.118, 0.118, 0.180);
const DANGER: Color = Color::rgb(0.953, 0.545, 0.659);

/// How long an armed `confirm` button waits for its second click.
const CONFIRM_WINDOW: Duration = Duration::from_secs(3);
/// State commands that take longer than this are killed and ignored.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
/// Downloading a track's artwork may take longer (curl itself stops at 5s).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(6);
/// How often state is re-read while the panel is open.
const OPEN_POLL: Duration = Duration::from_secs(1);
/// After the user moves a slider, ignore its `get` output for this long so a
/// stale reading cannot make the knob jump back mid-drag.
const SLIDER_SETTLE: Duration = Duration::from_millis(1500);
/// One wheel tick on a slider.
const SCROLL_STEP: f64 = 0.05;

#[derive(Debug, Clone, Default, PartialEq)]
struct TileState {
    active: bool,
    subtitle: Option<String>,
    /// Output of the `level` command, 0.0 to 1.0.
    level: Option<f64>,
    /// The tile's click command is still running; the worker leaves its
    /// (optimistically flipped) state alone until it finishes.
    busy: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct SliderState {
    /// 0.0 to 1.0, or `None` while the `get` command fails (slider hidden).
    value: Option<f64>,
    set_at: Option<Instant>,
}

/// The sub-page currently shown in place of the main page.
struct OpenPage {
    kind: PageKind,
    /// Command for the page's "settings" entry (the arrow's `menu_command`).
    settings: Option<String>,
    built: BuiltPage,
    /// Commands started from this page that are still running. The worker
    /// leaves the (optimistically updated) rows alone until they finish.
    pending: usize,
    /// Volumes the user just set, kept over stale readings for a moment.
    volumes: HashMap<VolumeTarget, (f64, Instant)>,
    /// A `confirm` row waiting for its second click, and since when.
    armed: Option<(usize, Instant)>,
}

impl OpenPage {
    fn new(kind: PageKind, settings: Option<String>, built: BuiltPage) -> Self {
        Self {
            kind,
            settings,
            built,
            pending: 0,
            volumes: HashMap::new(),
            armed: None,
        }
    }
}

#[derive(Default)]
struct Shared {
    tiles: Vec<TileState>,
    sliders: Vec<SliderState>,
    page: Option<OpenPage>,
    media: Option<MediaState>,
    /// When the user last pressed a media control.
    media_hold: Option<Instant>,
    /// Artwork of the current track, keyed by its URL so it is loaded once
    /// per track (`None` inside: it could not be loaded).
    art: Option<(String, Option<Arc<IconData>>)>,
}

enum Msg {
    Refresh,
    SetSlider(usize, f64),
    SetVolume(VolumeTarget, f64),
    /// A `playerctl ... position` command; only the latest one runs.
    Seek(String),
}

pub struct PanelModule {
    icon: String,
    chrome: ModuleChrome,
    style: PanelStyle,
    buttons: Vec<PanelButtonConfig>,
    sliders: Vec<PanelSliderConfig>,
    tiles: Vec<PanelTileConfig>,
    open: Arc<AtomicBool>,
    anchor_x: f64,
    /// Button waiting for its confirmation click, and since when.
    armed: Option<(usize, Instant)>,
    shared: Arc<Mutex<Shared>>,
    tx: Sender<Msg>,
}

impl PanelModule {
    pub fn new(chrome: ModuleChrome, config: &ModuleConfig) -> Self {
        let buttons = config.buttons.clone().unwrap_or_default();
        let sliders = config.sliders.clone().unwrap_or_default();
        let tiles = config.tiles.clone().unwrap_or_default();
        if buttons.is_empty() && sliders.is_empty() && tiles.is_empty() {
            log::warn!("[panel] no buttons, sliders or tiles configured; the panel will be empty");
        }
        let pages = sliders
            .iter()
            .map(|s| &s.page)
            .chain(tiles.iter().map(|t| &t.page));
        for page in pages.flatten() {
            if PageKind::parse(page).is_none() {
                log::warn!(
                    "[panel] unknown page '{page}' (expected audio-output, audio-input, wifi or bluetooth)"
                );
            }
        }

        let style = PanelStyle {
            width: config
                .panel_width
                .filter(|w| w.is_finite())
                .unwrap_or(DEFAULT_WIDTH)
                .max(MIN_WIDTH),
            corner_radius: CORNER_RADIUS,
            background: resolve_color(
                config.panel_background.as_deref(),
                BACKGROUND,
                "module.panel.panel_background",
            ),
            border: resolve_color(
                config.panel_border_color.as_deref(),
                BORDER,
                "module.panel.panel_border_color",
            ),
            surface: resolve_color(
                config.surface_color.as_deref(),
                SURFACE,
                "module.panel.surface_color",
            ),
            accent: resolve_color(
                config.accent_color.as_deref(),
                ACCENT,
                "module.panel.accent_color",
            ),
            on_accent: resolve_color(
                config.accent_text_color.as_deref(),
                ACCENT_TEXT,
                "module.panel.accent_text_color",
            ),
            danger: resolve_color(
                config.critical_color.as_deref(),
                DANGER,
                "module.panel.critical_color",
            ),
        };

        let shared = Arc::new(Mutex::new(Shared {
            tiles: vec![TileState::default(); tiles.len()],
            sliders: vec![SliderState::default(); sliders.len()],
            ..Default::default()
        }));
        let open = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        {
            let tiles = tiles.clone();
            let sliders = sliders.clone();
            let shared = shared.clone();
            let open = open.clone();
            let media = config.media.unwrap_or(false);
            std::thread::Builder::new()
                .name("panel-state".into())
                .spawn(move || {
                    let sources = Sources {
                        tiles: &tiles,
                        sliders: &sliders,
                        media,
                    };
                    state_worker(&sources, &shared, &open, &rx)
                })
                .map_err(|e| log::error!("[panel] failed to start state worker: {e}"))
                .ok();
        }

        Self {
            icon: config
                .icon
                .clone()
                .filter(|icon| !icon.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_ICON.to_string()),
            chrome,
            style,
            buttons,
            sliders,
            tiles,
            open,
            anchor_x: 0.0,
            armed: None,
            shared,
            tx,
        }
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    fn close(&mut self) {
        self.open.store(false, Ordering::Relaxed);
        self.armed = None;
        self.shared.lock().unwrap().page = None;
    }

    fn armed_button(&self) -> Option<usize> {
        self.armed
            .filter(|(_, at)| at.elapsed() < CONFIRM_WINDOW)
            .map(|(i, _)| i)
    }

    /// Config indices of the sliders currently shown (their `get` works).
    /// Panel slider targets index into this list.
    fn visible_sliders(shared: &Shared) -> Vec<usize> {
        shared
            .sliders
            .iter()
            .enumerate()
            .filter(|(_, s)| s.value.is_some())
            .map(|(i, _)| i)
            .collect()
    }

    fn build_view(&self) -> PanelView {
        let shared = self.shared.lock().unwrap();
        let armed = self.armed_button();

        let buttons = self
            .buttons
            .iter()
            .enumerate()
            .map(|(i, b)| PanelButton {
                icon: b.icon.clone(),
                align_right: b.align.as_deref().map(str::trim) == Some("right"),
                armed: armed == Some(i),
            })
            .collect();

        let sliders = Self::visible_sliders(&shared)
            .into_iter()
            .map(|i| PanelSlider {
                icon: self.sliders[i].icon.clone(),
                value: shared.sliders[i].value.unwrap_or(0.0),
                has_menu: has_menu(&self.sliders[i].page, &self.sliders[i].menu_command),
            })
            .collect();

        let tiles = self
            .tiles
            .iter()
            .zip(&shared.tiles)
            .map(|(cfg, state)| PanelTile {
                icon: tile_icon(cfg, state),
                label: cfg.label.clone(),
                subtitle: state.subtitle.clone(),
                active: state.active,
                has_menu: has_menu(&cfg.page, &cfg.menu_command),
            })
            .collect();

        let art = shared.art.as_ref().and_then(|(url, art)| {
            let current = shared.media.as_ref()?.art_url.as_ref()?;
            (current == url).then(|| art.clone()).flatten()
        });
        let media = shared.media.as_ref().map(|m| PanelMedia {
            icon: m.icon().to_string(),
            art: art.clone(),
            title: m.title.clone(),
            artist: m.artist.clone(),
            playing: m.playing,
            progress: m.progress(),
        });

        PanelView {
            buttons,
            media,
            sliders,
            tiles,
            page: shared.page.as_ref().map(|p| p.built.page.clone()),
            style: self.style.clone(),
        }
    }

    fn click_button(&mut self, index: usize) {
        let Some(button) = self.buttons.get(index) else {
            return;
        };
        if let Some(items) = &button.items {
            let title = button.label.as_deref().unwrap_or("Menu");
            let built = pages::custom(title, items);
            self.shared.lock().unwrap().page =
                Some(OpenPage::new(PageKind::Custom(index), None, built));
            return;
        }
        if button.confirm && self.armed_button() != Some(index) {
            self.armed = Some((index, Instant::now()));
            return;
        }
        spawn_detached(&button.command);
        self.close();
    }

    /// Previous / play-pause / next on the media card.
    fn media_control(&mut self, action: &str) {
        let mut shared = self.shared.lock().unwrap();
        let Some(media) = shared.media.as_mut() else {
            return;
        };
        let command = media.command(action);
        if action == "play-pause" {
            media.playing = !media.playing;
        }
        shared.media_hold = Some(Instant::now());
        drop(shared);
        self.run_tracked(&command);
    }

    fn seek_media(&mut self, hit: &PopupHit, button: MouseButton) {
        if button != MouseButton::Left {
            return;
        }
        let mut shared = self.shared.lock().unwrap();
        let Some(media) = shared.media.as_mut() else {
            return;
        };
        let Some(command) = media.seek_command(hit.fraction) else {
            return;
        };
        if let Some(length) = media.length {
            media.position = hit.fraction.clamp(0.0, 1.0) * length;
        }
        shared.media_hold = Some(Instant::now());
        drop(shared);
        self.tx.send(Msg::Seek(command)).ok();
    }

    /// Run a command, then refresh once it has finished.
    fn run_tracked(&self, command: &str) {
        if let Some(mut child) = spawn_shell(command, Stdio::null()) {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let _ = child.wait();
                tx.send(Msg::Refresh).ok();
            });
        }
    }

    fn move_slider(&mut self, visible_index: usize, hit: &PopupHit, button: MouseButton) {
        let mut shared = self.shared.lock().unwrap();
        let Some(index) = Self::visible_sliders(&shared).get(visible_index).copied() else {
            return;
        };
        let current = shared.sliders[index].value.unwrap_or(0.0);
        let Some(value) = slider_value(current, hit, button) else {
            return;
        };
        shared.sliders[index] = SliderState {
            value: Some(value),
            set_at: Some(Instant::now()),
        };
        drop(shared);
        self.tx.send(Msg::SetSlider(index, value)).ok();
    }

    fn click_slider_menu(&mut self, visible_index: usize) {
        let index = Self::visible_sliders(&self.shared.lock().unwrap())
            .get(visible_index)
            .copied();
        if let Some(slider) = index.and_then(|i| self.sliders.get(i)).cloned() {
            self.open_menu(slider.page.as_deref(), slider.menu_command);
        }
    }

    /// An arrow was clicked: open its page, or run its command.
    fn open_menu(&mut self, page: Option<&str>, command: Option<String>) {
        let command = command.filter(|c| !c.trim().is_empty());
        if let Some(kind) = page.and_then(PageKind::parse) {
            self.shared.lock().unwrap().page =
                Some(OpenPage::new(kind, command, pages::loading(kind)));
            self.tx.send(Msg::Refresh).ok();
        } else if let Some(command) = command {
            spawn_detached(&command);
            self.close();
        }
    }

    fn click_page_item(&mut self, row: usize) {
        let mut shared = self.shared.lock().unwrap();
        let Some(page) = shared.page.as_mut() else {
            return;
        };
        let Some(click) = page.built.actions.get(row).and_then(|a| a.click.clone()) else {
            return;
        };
        if click.confirm {
            let armed = page
                .armed
                .is_some_and(|(r, at)| r == row && at.elapsed() < CONFIRM_WINDOW);
            if !armed {
                page.armed = Some((row, Instant::now()));
                if let Some(PageRow::Item { detail, .. }) = page.built.page.rows.get_mut(row) {
                    *detail = Some("Click again".into());
                }
                return;
            }
        }
        if click.close_panel {
            drop(shared);
            spawn_detached(&click.command);
            self.close();
            return;
        }
        if click.select {
            select_row(&mut page.built, row);
        }
        if let (Some(label), Some(PageRow::Item { detail, .. })) =
            (&click.busy_label, page.built.page.rows.get_mut(row))
        {
            *detail = Some(label.clone());
        }
        page.pending += 1;
        drop(shared);
        self.run_page_command(&click.command);
    }

    fn move_page_slider(&mut self, row: usize, hit: &PopupHit, button: MouseButton) {
        let mut shared = self.shared.lock().unwrap();
        let Some(page) = shared.page.as_mut() else {
            return;
        };
        let Some(target) = page.built.actions.get(row).and_then(|a| a.volume) else {
            return;
        };
        let Some(PageRow::Slider { value, .. }) = page.built.page.rows.get_mut(row) else {
            return;
        };
        let Some(new_value) = slider_value(*value, hit, button) else {
            return;
        };
        *value = new_value;
        page.volumes.insert(target, (new_value, Instant::now()));
        drop(shared);
        self.tx.send(Msg::SetVolume(target, new_value)).ok();
    }

    fn toggle_page_mute(&mut self, row: usize) {
        let mut shared = self.shared.lock().unwrap();
        let Some(page) = shared.page.as_mut() else {
            return;
        };
        let Some(target) = page.built.actions.get(row).and_then(|a| a.volume) else {
            return;
        };
        if let Some(PageRow::Slider { muted, .. }) = page.built.page.rows.get_mut(row) {
            *muted = !*muted;
        }
        page.pending += 1;
        drop(shared);
        self.run_page_command(&target.toggle_mute_command());
    }

    /// Run a command started from the page, then let the worker re-read the
    /// page once it is done.
    fn run_page_command(&self, command: &str) {
        let done = |shared: &Mutex<Shared>| {
            if let Some(page) = shared.lock().unwrap().page.as_mut() {
                page.pending = page.pending.saturating_sub(1);
            }
        };
        let Some(mut child) = spawn_shell(command, Stdio::null()) else {
            done(&self.shared);
            return;
        };
        let shared = self.shared.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            done(&shared);
            tx.send(Msg::Refresh).ok();
        });
    }

    fn click_tile(&mut self, index: usize) {
        let Some(tile) = self.tiles.get(index) else {
            return;
        };
        if tile.state.is_none() {
            spawn_detached(&tile.command);
            return;
        }

        // Flip right away so the click feels instant; the worker re-reads
        // the real state once the command has finished.
        {
            let mut shared = self.shared.lock().unwrap();
            let state = &mut shared.tiles[index];
            state.active = !state.active;
            state.busy = true;
        }
        let Some(mut child) = spawn_shell(&tile.command, Stdio::null()) else {
            self.shared.lock().unwrap().tiles[index].busy = false;
            return;
        };
        let shared = self.shared.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            shared.lock().unwrap().tiles[index].busy = false;
            tx.send(Msg::Refresh).ok();
        });
    }

    fn click_tile_menu(&mut self, index: usize) {
        if let Some(tile) = self.tiles.get(index).cloned() {
            self.open_menu(tile.page.as_deref(), tile.menu_command);
        }
    }
}

impl Module for PanelModule {
    fn update(&mut self) {
        if self.armed.is_some() && self.armed_button().is_none() {
            self.armed = None;
        }
    }

    fn view(&self) -> ModuleView {
        let mut view = self.chrome.apply(ModuleView {
            text: self.icon.clone(),
            style: TextStyle::default(),
            ..Default::default()
        });
        if self.is_open() {
            view.style.color = self.style.accent;
        }
        view
    }

    fn click(&mut self, event: ClickEvent) {
        if event.button != MouseButton::Left {
            return;
        }
        if self.is_open() {
            self.close();
            return;
        }
        // Centre the panel under the icon (the popup code keeps it on screen).
        self.anchor_x = event.bar_x - event.x + event.module_width / 2.0;
        self.armed = None;
        self.open.store(true, Ordering::Relaxed);
        self.tx.send(Msg::Refresh).ok();
    }

    fn popup(&self) -> Option<Popup> {
        self.is_open().then(|| {
            Popup::Panel(Box::new(PanelPopup {
                anchor_x: self.anchor_x,
                view: self.build_view(),
            }))
        })
    }

    fn popup_click(&mut self, hit: PopupHit, button: MouseButton) {
        let HitTarget::Panel(target) = hit.target else {
            return;
        };
        match target {
            PanelTarget::Slider(i) => return self.move_slider(i, &hit, button),
            PanelTarget::PageSlider(i) => return self.move_page_slider(i, &hit, button),
            PanelTarget::MediaSeek => return self.seek_media(&hit, button),
            _ => {}
        }
        if button != MouseButton::Left {
            return;
        }
        match target {
            PanelTarget::Button(i) => self.click_button(i),
            PanelTarget::SliderMenu(i) => self.click_slider_menu(i),
            PanelTarget::Tile(i) => self.click_tile(i),
            PanelTarget::TileMenu(i) => self.click_tile_menu(i),
            PanelTarget::Back => self.shared.lock().unwrap().page = None,
            PanelTarget::PageItem(i) => self.click_page_item(i),
            PanelTarget::PageSliderIcon(i) => self.toggle_page_mute(i),
            PanelTarget::MediaPrevious => self.media_control("previous"),
            PanelTarget::MediaPlayPause => self.media_control("play-pause"),
            PanelTarget::MediaNext => self.media_control("next"),
            PanelTarget::Slider(_) | PanelTarget::PageSlider(_) | PanelTarget::MediaSeek => {}
        }
    }

    fn dismiss_popup(&mut self) {
        self.close();
    }
}

fn has_command(command: &Option<String>) -> bool {
    command.as_deref().is_some_and(|c| !c.trim().is_empty())
}

/// The icon a tile shows: `icon_off` while it is off, the `icons` entry for
/// its current level, or its plain `icon`.
fn tile_icon(cfg: &PanelTileConfig, state: &TileState) -> String {
    let off = cfg.state.is_some() && !state.active;
    if off && let Some(icon) = cfg.icon_off.as_ref().filter(|i| !i.is_empty()) {
        return icon.clone();
    }
    let icons = cfg.icons.as_deref().unwrap_or_default();
    match (state.level, icons.len()) {
        (Some(level), n) if n > 0 => {
            // Round up so any signal at all shows at least the first bar.
            let index = (level.clamp(0.0, 1.0) * (n - 1) as f64 - 1e-9)
                .ceil()
                .max(0.0);
            icons[(index as usize).min(n - 1)].clone()
        }
        _ => cfg.icon.clone(),
    }
}

/// Whether a slider or tile gets an arrow: it opens a known page or runs a
/// command.
fn has_menu(page: &Option<String>, command: &Option<String>) -> bool {
    page.as_deref().and_then(PageKind::parse).is_some() || has_command(command)
}

/// The value a slider takes for a click at `hit` (left button) or a wheel
/// tick, snapped to whole percents (all a `set` command receives anyway).
fn slider_value(current: f64, hit: &PopupHit, button: MouseButton) -> Option<f64> {
    let value = match button {
        MouseButton::Left => hit.fraction,
        MouseButton::ScrollUp => current + SCROLL_STEP,
        MouseButton::ScrollDown => current - SCROLL_STEP,
        _ => return None,
    };
    Some((value.clamp(0.0, 1.0) * 100.0).round() / 100.0)
}

/// Check `row` and uncheck every other selectable row (a radio group).
fn select_row(built: &mut BuiltPage, row: usize) {
    for (i, (entry, actions)) in built.page.rows.iter_mut().zip(&built.actions).enumerate() {
        if !actions.click.as_ref().is_some_and(|c| c.select) {
            continue;
        }
        match entry {
            PageRow::Item { checked, .. } => *checked = i == row,
            PageRow::Slider { checked, .. } => *checked = Some(i == row),
            _ => {}
        }
    }
}

/// Runs state commands: once at startup, every `OPEN_POLL` while the panel
/// is open, and whenever the module asks. Slider writes are coalesced so a
/// drag only runs `set` for the latest value.
/// What the worker reads state from.
struct Sources<'a> {
    tiles: &'a [PanelTileConfig],
    sliders: &'a [PanelSliderConfig],
    /// Read `playerctl` for the media card.
    media: bool,
}

fn state_worker(sources: &Sources, shared: &Mutex<Shared>, open: &AtomicBool, rx: &Receiver<Msg>) {
    let sliders = sources.sliders;
    refresh(sources, shared);
    loop {
        let mut refresh_needed = false;
        let mut pending: Vec<Option<f64>> = vec![None; sliders.len()];
        let mut volumes: HashMap<VolumeTarget, f64> = HashMap::new();
        let mut seek: Option<String> = None;
        let mut handle = |msg: Msg, refresh_needed: &mut bool| match msg {
            Msg::Refresh => *refresh_needed = true,
            Msg::SetSlider(i, value) => {
                if let Some(slot) = pending.get_mut(i) {
                    *slot = Some(value);
                }
            }
            Msg::SetVolume(target, value) => {
                volumes.insert(target, value);
            }
            Msg::Seek(command) => seek = Some(command),
        };

        match rx.recv_timeout(OPEN_POLL) {
            Ok(msg) => handle(msg, &mut refresh_needed),
            Err(RecvTimeoutError::Timeout) => {
                if !open.load(Ordering::Relaxed) {
                    continue;
                }
                refresh_needed = true;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(msg) = rx.try_recv() {
            handle(msg, &mut refresh_needed);
        }

        for (slider, value) in sliders.iter().zip(&pending) {
            if let Some(value) = value {
                let percent = (value * 100.0).round() as u32;
                let command = slider.set.replace("{value}", &percent.to_string());
                run_with_timeout(&command);
            }
        }
        for (target, value) in volumes {
            run_with_timeout(&target.set_volume_command((value * 100.0).round() as u32));
        }
        if let Some(command) = seek {
            run_with_timeout(&command);
        }
        if refresh_needed {
            refresh(sources, shared);
        }
    }
}

/// Rebuild the open sub-page, if any. Runs its commands without holding the
/// lock and drops the result if the page changed or a click is in flight.
fn refresh_page(shared: &Mutex<Shared>) {
    let Some((kind, settings)) = shared
        .lock()
        .unwrap()
        .page
        .as_ref()
        .map(|p| (p.kind, p.settings.clone()))
    else {
        return;
    };
    let mut built = pages::build(kind, settings.as_deref(), &run_with_timeout);

    let mut shared = shared.lock().unwrap();
    let Some(page) = shared.page.as_mut().filter(|p| p.kind == kind) else {
        return;
    };
    // Config pages never change, and rebuilding would drop a pending
    // "Click again" confirmation.
    if matches!(kind, PageKind::Custom(_)) {
        return;
    }
    if page.pending > 0 {
        return;
    }
    page.volumes
        .retain(|_, (_, at)| at.elapsed() < SLIDER_SETTLE);
    for (row, actions) in built.page.rows.iter_mut().zip(&built.actions) {
        if let (PageRow::Slider { value, .. }, Some(target)) = (row, actions.volume)
            && let Some((user_value, _)) = page.volumes.get(&target)
        {
            *value = *user_value;
        }
    }
    page.built = built;
}

/// What one refresh read for a tile. `None` means "keep the last value"
/// (the command is not configured or timed out).
struct TileReading {
    active: Option<bool>,
    subtitle: Option<Option<String>>,
    level: Option<Option<f64>>,
}

/// Re-read what is playing, unless the user just pressed a control (the
/// player may not have caught up yet).
fn refresh_media(shared: &Mutex<Shared>) {
    let state = run_with_timeout(media::QUERY).and_then(
        |(ok, out)| {
            if ok { media::parse(&out) } else { None }
        },
    );

    // Load the artwork once per track, outside the lock (it may download).
    let url = state.as_ref().and_then(|s| s.art_url.clone());
    let cached = shared.lock().unwrap().art.as_ref().map(|(u, _)| u.clone());
    if let Some(url) = url.filter(|url| cached.as_ref() != Some(url)) {
        let art = media::load_art(&url, |url, destination| {
            run_for(&media::download_command(url, destination), DOWNLOAD_TIMEOUT)
                .is_some_and(|(ok, _)| ok)
        });
        if art.is_none() {
            log::debug!("[panel] no artwork from {url}");
        }
        shared.lock().unwrap().art = Some((url, art.map(Arc::new)));
    }

    let mut shared = shared.lock().unwrap();
    if shared
        .media_hold
        .is_some_and(|at| at.elapsed() < SLIDER_SETTLE)
    {
        return;
    }
    shared.media_hold = None;
    shared.media = state;
}

fn refresh(sources: &Sources, shared: &Mutex<Shared>) {
    let (tiles, sliders) = (sources.tiles, sources.sliders);
    refresh_page(shared);
    if sources.media {
        refresh_media(shared);
    }
    // Run every command without holding the lock, then publish the results.
    let tile_results: Vec<TileReading> = tiles
        .iter()
        .map(|tile| {
            let level = tile
                .level
                .as_deref()
                .and_then(run_with_timeout)
                .map(|(ok, out)| ok.then(|| parse_percent(&out)).flatten());
            let active = tile
                .state
                .as_deref()
                .and_then(run_with_timeout)
                .map(|(ok, _)| ok);
            let subtitle = tile
                .subtitle
                .as_deref()
                .and_then(run_with_timeout)
                .map(|(_, out)| first_line(&out));
            TileReading {
                active,
                subtitle,
                level,
            }
        })
        .collect();
    let slider_results: Vec<Option<Option<f64>>> = sliders
        .iter()
        .map(|slider| {
            run_with_timeout(&slider.get).map(|(ok, out)| ok.then(|| parse_percent(&out)).flatten())
        })
        .collect();

    let mut shared = shared.lock().unwrap();
    for (state, reading) in shared.tiles.iter_mut().zip(tile_results) {
        let TileReading {
            active,
            subtitle,
            level,
        } = reading;
        if let Some(level) = level {
            state.level = level;
        }
        if state.busy {
            continue;
        }
        if let Some(active) = active {
            state.active = active;
        }
        if let Some(subtitle) = subtitle {
            state.subtitle = subtitle;
        }
    }
    for (state, result) in shared.sliders.iter_mut().zip(slider_results) {
        let settling = state.set_at.is_some_and(|at| at.elapsed() < SLIDER_SETTLE);
        // `None` means the command timed out: keep the last value.
        if let (false, Some(value)) = (settling, result) {
            state.value = value;
        }
    }
}

fn first_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// First number in `output`, read as a percentage and returned as 0.0-1.0.
fn parse_percent(output: &str) -> Option<f64> {
    let start = output.find(|c: char| c.is_ascii_digit())?;
    let number: String = output[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let value: f64 = number.parse().ok()?;
    value.is_finite().then(|| (value / 100.0).clamp(0.0, 1.0))
}

fn spawn_shell(command: &str, stdout: Stdio) -> Option<Child> {
    Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::null())
        // Own process group, so a timeout can kill the whole pipeline.
        .process_group(0)
        .spawn()
        .map_err(|e| log::warn!("[panel] failed to run '{command}': {e}"))
        .ok()
}

/// Start a command and forget about it (a reaper thread collects it).
fn spawn_detached(command: &str) {
    if command.trim().is_empty() {
        return;
    }
    if let Some(mut child) = spawn_shell(command, Stdio::null()) {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Run a command and return (exited with 0, stdout), or `None` if it could
/// not start or ran longer than `COMMAND_TIMEOUT`.
fn run_with_timeout(command: &str) -> Option<(bool, String)> {
    run_for(command, COMMAND_TIMEOUT)
}

/// `run_with_timeout` with a custom time limit.
fn run_for(command: &str, timeout: Duration) -> Option<(bool, String)> {
    let mut child = spawn_shell(command, Stdio::piped())?;
    let mut stdout = child.stdout.take()?;
    // Read on a helper thread so a chatty command cannot fill the pipe and
    // stall while we wait for it to exit.
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        out
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                // SAFETY: plain kill(2) on the process group we created.
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.wait();
                log::debug!("[panel] '{command}' timed out");
                break None;
            }
        }
    };
    let out = reader.join().unwrap_or_default();
    status.map(|status| (status.success(), out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_percent_from_command_output() {
        assert_eq!(parse_percent("87\n"), Some(0.87));
        assert_eq!(parse_percent("50%"), Some(0.5));
        assert_eq!(parse_percent("Volume: 45.5"), Some(0.455));
        assert_eq!(parse_percent("150"), Some(1.0));
        assert_eq!(parse_percent("no number"), None);
    }

    #[test]
    fn first_line_skips_blank_lines() {
        assert_eq!(
            first_line("\n  home-wifi \nother"),
            Some("home-wifi".into())
        );
        assert_eq!(first_line("   \n"), None);
    }

    #[test]
    fn run_with_timeout_reports_status_and_output() {
        assert_eq!(run_with_timeout("echo hi"), Some((true, "hi\n".into())));
        assert_eq!(run_with_timeout("exit 3"), Some((false, String::new())));
    }

    #[test]
    fn run_with_timeout_kills_slow_commands() {
        let started = Instant::now();
        assert_eq!(run_with_timeout("sleep 10 | cat"), None);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    fn module(config: &str) -> PanelModule {
        let config: ModuleConfig = toml::from_str(config).unwrap();
        PanelModule::new(ModuleChrome::from_config(&config, (8.0, 8.0)), &config)
    }

    fn wait_for(module: &PanelModule, check: impl Fn(&Shared) -> bool) {
        let started = Instant::now();
        while !check(&module.shared.lock().unwrap()) {
            assert!(started.elapsed() < Duration::from_secs(5), "timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn open(module: &mut PanelModule) {
        module.click(ClickEvent {
            x: 5.0,
            bar_x: 105.0,
            screen_x: 105.0,
            module_width: 20.0,
            y: 5.0,
            bar_y: 5.0,
            screen_y: 5.0,
            button: MouseButton::Left,
        });
    }

    fn hit(target: PanelTarget, fraction: f64) -> PopupHit {
        PopupHit {
            index: 0,
            target: HitTarget::Panel(target),
            fraction,
        }
    }

    #[test]
    fn clicking_the_icon_toggles_the_panel() {
        let mut m = module("type = \"panel\"");
        assert!(m.popup().is_none());

        open(&mut m);
        let Some(Popup::Panel(panel)) = m.popup() else {
            panic!("panel should be open");
        };
        assert_eq!(panel.anchor_x, 110.0);

        open(&mut m);
        assert!(m.popup().is_none());
    }

    #[test]
    fn tiles_and_sliders_show_command_state() {
        let m = module(
            r#"
type = "panel"
[[tiles]]
label = "On"
icon = "a"
command = "true"
state = "true"
subtitle = "echo connected"
[[tiles]]
label = "Off"
icon = "b"
command = "true"
state = "false"
[[sliders]]
icon = "v"
get = "echo 40"
set = "true"
[[sliders]]
icon = "x"
get = "exit 1"
set = "true"
"#,
        );
        wait_for(&m, |s| s.sliders[0].value.is_some());

        let view = m.build_view();
        assert!(view.tiles[0].active);
        assert_eq!(view.tiles[0].subtitle.as_deref(), Some("connected"));
        assert!(!view.tiles[1].active);
        // The failing slider is hidden.
        assert_eq!(view.sliders.len(), 1);
        assert_eq!(view.sliders[0].value, 0.4);
    }

    #[test]
    fn confirm_buttons_need_two_clicks() {
        let dir = std::env::temp_dir().join(format!("beebar-panel-{}", std::process::id()));
        let marker = dir.join("ran");
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = module(&format!(
            "type = \"panel\"\n[[buttons]]\nicon = \"p\"\ncommand = \"touch '{}'\"\nconfirm = true\n",
            marker.display()
        ));
        open(&mut m);

        m.popup_click(hit(PanelTarget::Button(0), 0.0), MouseButton::Left);
        assert!(m.build_view().buttons[0].armed);
        assert!(m.popup().is_some(), "first click only arms the button");

        m.popup_click(hit(PanelTarget::Button(0), 0.0), MouseButton::Left);
        assert!(m.popup().is_none(), "second click runs it and closes");
        let started = Instant::now();
        while !marker.exists() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "command never ran"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn slider_clicks_and_scrolls_set_the_value() {
        let mut m = module(
            "type = \"panel\"\n[[sliders]]\nicon = \"v\"\nget = \"echo 50\"\nset = \"true\"\n",
        );
        wait_for(&m, |s| s.sliders[0].value.is_some());
        open(&mut m);

        m.popup_click(hit(PanelTarget::Slider(0), 0.234), MouseButton::Left);
        assert_eq!(m.build_view().sliders[0].value, 0.23);

        m.popup_click(hit(PanelTarget::Slider(0), 0.0), MouseButton::ScrollUp);
        assert_eq!(m.build_view().sliders[0].value, 0.28);

        // The stale `get` output (50) must not undo the user's change.
        m.tx.send(Msg::Refresh).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(m.build_view().sliders[0].value, 0.28);
    }

    #[test]
    fn tile_click_flips_immediately_and_keeps_panel_open() {
        let mut m = module(
            "type = \"panel\"\n[[tiles]]\nlabel = \"T\"\nicon = \"t\"\ncommand = \"sleep 0.3\"\nstate = \"false\"\n",
        );
        wait_for(&m, |s| !s.tiles[0].busy);
        open(&mut m);

        m.popup_click(hit(PanelTarget::Tile(0), 0.0), MouseButton::Left);
        assert!(m.build_view().tiles[0].active, "optimistic flip");
        assert!(m.popup().is_some());

        // Once the command finishes the real state (false) comes back.
        wait_for(&m, |s| !s.tiles[0].busy && !s.tiles[0].active);
    }

    fn page_module() -> PanelModule {
        let mut m = module(
            "type = \"panel\"\n[[tiles]]\nlabel = \"BT\"\nicon = \"b\"\ncommand = \"true\"\npage = \"bluetooth\"\nmenu_command = \"true\"\n",
        );
        open(&mut m);
        m
    }

    /// Put a fake page in place, with a click in flight so the worker does
    /// not replace it with real system state.
    fn inject_page(m: &PanelModule, rows: Vec<(PageRow, pages::RowActions)>) {
        let (rows, actions) = rows.into_iter().unzip();
        m.shared.lock().unwrap().page = Some(OpenPage {
            kind: PageKind::Bluetooth,
            settings: None,
            built: BuiltPage {
                page: crate::core::panel::PanelPage {
                    title: "Test".into(),
                    rows,
                },
                actions,
            },
            pending: 1,
            volumes: HashMap::new(),
            armed: None,
        });
    }

    fn device(label: &str, checked: bool) -> (PageRow, pages::RowActions) {
        (
            PageRow::Slider {
                icon: "d".into(),
                label: label.into(),
                value: 0.5,
                muted: false,
                checked: Some(checked),
            },
            pages::RowActions {
                click: Some(pages::Click {
                    command: "true".into(),
                    select: true,
                    ..Default::default()
                }),
                volume: Some(VolumeTarget {
                    kind: pages::StreamKind::Sink,
                    index: 1,
                }),
            },
        )
    }

    #[test]
    fn arrow_opens_page_and_back_returns() {
        let mut m = page_module();
        assert!(m.build_view().tiles[0].has_menu);

        m.popup_click(hit(PanelTarget::TileMenu(0), 0.0), MouseButton::Left);
        let page = m.build_view().page.expect("page should be open");
        assert_eq!(page.title, "Bluetooth");
        assert!(m.popup().is_some(), "opening a page keeps the panel open");

        m.popup_click(hit(PanelTarget::Back, 0.0), MouseButton::Left);
        assert!(m.build_view().page.is_none());
        assert!(m.popup().is_some());
    }

    #[test]
    fn closing_the_panel_forgets_the_page() {
        let mut m = page_module();
        m.popup_click(hit(PanelTarget::TileMenu(0), 0.0), MouseButton::Left);
        m.dismiss_popup();
        open(&mut m);
        assert!(m.build_view().page.is_none());
    }

    #[test]
    fn selecting_a_device_moves_the_check_mark() {
        let mut m = page_module();
        inject_page(&m, vec![device("A", true), device("B", false)]);

        m.popup_click(hit(PanelTarget::PageItem(1), 0.0), MouseButton::Left);
        let rows = m.build_view().page.unwrap().rows;
        let checks: Vec<Option<bool>> = rows
            .iter()
            .map(|r| match r {
                PageRow::Slider { checked, .. } => *checked,
                _ => None,
            })
            .collect();
        assert_eq!(checks, [Some(false), Some(true)]);
    }

    #[test]
    fn page_sliders_and_mute_update_right_away() {
        let mut m = page_module();
        inject_page(&m, vec![device("A", true)]);

        m.popup_click(hit(PanelTarget::PageSlider(0), 0.3), MouseButton::Left);
        m.popup_click(hit(PanelTarget::PageSlider(0), 0.0), MouseButton::ScrollUp);
        m.popup_click(hit(PanelTarget::PageSliderIcon(0), 0.0), MouseButton::Left);
        let PageRow::Slider { value, muted, .. } = &m.build_view().page.unwrap().rows[0] else {
            panic!("expected slider");
        };
        assert_eq!(*value, 0.35);
        assert!(*muted);
    }

    #[test]
    fn busy_label_shows_while_command_runs() {
        let mut m = page_module();
        inject_page(
            &m,
            vec![(
                PageRow::Item {
                    icon: "h".into(),
                    label: "Headphones".into(),
                    detail: None,
                    checked: false,
                },
                pages::RowActions {
                    click: Some(pages::Click {
                        command: "sleep 0.2".into(),
                        busy_label: Some("Connecting".into()),
                        ..Default::default()
                    }),
                    volume: None,
                },
            )],
        );

        m.popup_click(hit(PanelTarget::PageItem(0), 0.0), MouseButton::Left);
        let PageRow::Item { detail, .. } = &m.build_view().page.unwrap().rows[0] else {
            panic!("expected item");
        };
        assert_eq!(detail.as_deref(), Some("Connecting"));
    }

    #[test]
    fn tile_icon_follows_level_and_state() {
        let cfg: PanelTileConfig = toml::from_str(
            "label = \"Wi-Fi\"\nicon = \"W\"\ncommand = \"true\"\nstate = \"true\"\nicons = [\"0\", \"1\", \"2\", \"3\", \"4\"]\nicon_off = \"off\"\n",
        )
        .unwrap();
        let icon = |active, level: Option<f64>| {
            tile_icon(
                &cfg,
                &TileState {
                    active,
                    level,
                    ..Default::default()
                },
            )
        };
        assert_eq!(icon(true, Some(0.59)), "3");
        assert_eq!(icon(true, Some(1.0)), "4");
        assert_eq!(icon(true, Some(0.75)), "3");
        assert_eq!(icon(true, Some(0.76)), "4");
        assert_eq!(icon(true, Some(0.1)), "1");
        assert_eq!(icon(true, Some(0.0)), "0");
        // No level reading (not connected): the plain icon.
        assert_eq!(icon(true, None), "W");
        assert_eq!(icon(false, Some(0.9)), "off");
    }

    #[test]
    fn button_with_items_opens_a_menu_page_with_confirmation() {
        let dir = std::env::temp_dir().join(format!("beebar-menu-{}", std::process::id()));
        let marker = dir.join("ran");
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = module(&format!(
            "type = \"panel\"\n[[buttons]]\nicon = \"p\"\nlabel = \"Power\"\n\
             [[buttons.items]]\nicon = \"o\"\nlabel = \"Power Off\"\ncommand = \"touch '{}'\"\nconfirm = true\n",
            marker.display()
        ));
        open(&mut m);

        m.popup_click(hit(PanelTarget::Button(0), 0.0), MouseButton::Left);
        let page = m.build_view().page.expect("menu page");
        assert_eq!(page.title, "Power");

        // First click only asks for confirmation.
        m.popup_click(hit(PanelTarget::PageItem(0), 0.0), MouseButton::Left);
        let PageRow::Item { detail, .. } = &m.build_view().page.unwrap().rows[0] else {
            panic!("expected item");
        };
        assert_eq!(detail.as_deref(), Some("Click again"));
        assert!(m.popup().is_some());

        // The worker must not wipe the confirmation of a config page.
        m.tx.send(Msg::Refresh).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(m.build_view().page.is_some());

        m.popup_click(hit(PanelTarget::PageItem(0), 0.0), MouseButton::Left);
        assert!(m.popup().is_none(), "second click runs it and closes");
        let started = Instant::now();
        while !marker.exists() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "command never ran"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn media_card_shows_and_controls_the_player() {
        // `media` is off in this config, so the worker leaves our fake
        // player alone.
        let mut m = module("type = \"panel\"");
        m.shared.lock().unwrap().media = Some(MediaState {
            instance: "nonexistent-player".into(),
            playing: false,
            title: "Song".into(),
            artist: Some("Artist".into()),
            position: 30.0,
            length: Some(120.0),
            art_url: None,
        });
        open(&mut m);

        let media = m.build_view().media.expect("media card");
        assert_eq!(media.title, "Song");
        assert_eq!(media.progress, Some(0.25));
        assert!(!media.playing);

        m.popup_click(hit(PanelTarget::MediaPlayPause, 0.0), MouseButton::Left);
        assert!(m.build_view().media.unwrap().playing, "optimistic play");

        m.popup_click(hit(PanelTarget::MediaSeek, 0.5), MouseButton::Left);
        assert_eq!(m.build_view().media.unwrap().progress, Some(0.5));
        // Wheel ticks do not seek.
        m.popup_click(hit(PanelTarget::MediaSeek, 0.9), MouseButton::ScrollUp);
        assert_eq!(m.build_view().media.unwrap().progress, Some(0.5));
        assert!(m.popup().is_some());
    }

    #[test]
    fn artwork_is_only_shown_for_its_own_track() {
        let m = module("type = \"panel\"");
        let icon = Arc::new(IconData {
            pixels: vec![0; 4],
            width: 1,
            height: 1,
        });
        {
            let mut shared = m.shared.lock().unwrap();
            shared.media = Some(MediaState {
                instance: "p".into(),
                playing: true,
                title: "Song".into(),
                artist: None,
                position: 0.0,
                length: None,
                art_url: Some("file:///a.png".into()),
            });
            shared.art = Some(("file:///a.png".into(), Some(icon.clone())));
        }
        assert_eq!(m.build_view().media.unwrap().art, Some(icon));

        // A new track whose artwork is not loaded yet: show the logo, not the
        // previous track's cover.
        m.shared.lock().unwrap().media.as_mut().unwrap().art_url = Some("file:///b.png".into());
        assert_eq!(m.build_view().media.unwrap().art, None);
    }
}
