use std::collections::HashMap;

use zbus::{
    blocking::{Connection, Proxy},
    proxy,
    zvariant::OwnedValue,
};

use super::{Module, ModuleChrome, ModuleView, char_len};
use crate::core::event::{ClickEvent, MouseButton};
use crate::renderer::primitives::TextStyle;

const DBUS_DESTINATION: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";
const DBUS_INTERFACE: &str = "org.freedesktop.DBus";
const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const DEFAULT_ICON: &str = "";
const DEFAULT_UNAVAILABLE_ICON: &str = "󰎈";
const DEFAULT_FORMAT: &str = "{icon} {track} {buttons}";
const NO_MEDIA_TEXT: &str = "no media";
const UNAVAILABLE_TEXT: &str = "unavailable";
const PREVIOUS_ICON: &str = "󰒮";
const STOP_ICON: &str = "󰓛";
const PLAY_ICON: &str = "󰐊";
const PAUSE_ICON: &str = "󰏤";
const NEXT_ICON: &str = "󰒭";
const BUTTON_GAP_CHARS: usize = 1;

#[proxy(
    interface = "org.mpris.MediaPlayer2",
    default_path = "/org/mpris/MediaPlayer2",
    gen_blocking = true
)]
trait MprisRoot {
    #[zbus(property)]
    fn identity(&self) -> zbus::Result<String>;
}

#[proxy(
    interface = "org.mpris.MediaPlayer2.Player",
    default_path = "/org/mpris/MediaPlayer2",
    gen_blocking = true
)]
trait MprisPlayer {
    #[zbus(property)]
    fn playback_status(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;

    #[zbus(property)]
    fn can_go_next(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn can_go_previous(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn can_play(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn can_pause(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn can_control(&self) -> zbus::Result<bool>;

    fn next(&self) -> zbus::Result<()>;
    fn previous(&self) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;
    fn play_pause(&self) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

impl PlaybackStatus {
    fn from_raw(value: &str) -> Self {
        match value.trim() {
            "Playing" => Self::Playing,
            "Paused" => Self::Paused,
            "Stopped" => Self::Stopped,
            _ => Self::Unknown,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Playing => "playing",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
            Self::Unknown => "unknown",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Playing => 3,
            Self::Paused => 2,
            Self::Stopped => 1,
            Self::Unknown => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlayerState {
    service_name: String,
    player_name: String,
    title: String,
    artist: String,
    status: PlaybackStatus,
    can_control: bool,
    can_go_previous: bool,
    can_go_next: bool,
    can_toggle_playback: bool,
}

impl PlayerState {
    fn track_text(&self) -> String {
        if !self.artist.is_empty() && !self.title.is_empty() {
            format!("{} - {}", self.artist, self.title)
        } else if !self.title.is_empty() {
            self.title.clone()
        } else if !self.player_name.is_empty() {
            self.player_name.clone()
        } else {
            NO_MEDIA_TEXT.to_string()
        }
    }

    fn supports_action(&self, action: ControlAction) -> bool {
        match action {
            ControlAction::Previous => self.can_control && self.can_go_previous,
            ControlAction::Stop => self.can_control,
            ControlAction::TogglePlayback => self.can_control && self.can_toggle_playback,
            ControlAction::Next => self.can_control && self.can_go_next,
        }
    }

    fn has_visible_controls(&self) -> bool {
        self.can_control || self.can_go_previous || self.can_go_next || self.can_toggle_playback
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlaybackViewState {
    Unavailable,
    NoMedia,
    Player(PlayerState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlAction {
    Previous,
    Stop,
    TogglePlayback,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlaybackButtons {
    previous: &'static str,
    stop: &'static str,
    toggle: &'static str,
    next: &'static str,
}

impl PlaybackButtons {
    fn from_status(status: PlaybackStatus) -> Self {
        Self {
            previous: PREVIOUS_ICON,
            stop: STOP_ICON,
            toggle: match status {
                PlaybackStatus::Playing => PAUSE_ICON,
                PlaybackStatus::Paused | PlaybackStatus::Stopped | PlaybackStatus::Unknown => {
                    PLAY_ICON
                }
            },
            next: NEXT_ICON,
        }
    }

    fn text(self) -> String {
        format!(
            "{} {} {} {}",
            self.previous, self.stop, self.toggle, self.next
        )
    }

    fn total_chars(self) -> usize {
        char_len(self.previous)
            + BUTTON_GAP_CHARS
            + char_len(self.stop)
            + BUTTON_GAP_CHARS
            + char_len(self.toggle)
            + BUTTON_GAP_CHARS
            + char_len(self.next)
    }

    fn action_for_char_offset(self, offset: usize) -> Option<ControlAction> {
        let previous_end = char_len(self.previous);
        if offset < previous_end {
            return Some(ControlAction::Previous);
        }

        let stop_start = previous_end + BUTTON_GAP_CHARS;
        let stop_end = stop_start + char_len(self.stop);
        if (stop_start..stop_end).contains(&offset) {
            return Some(ControlAction::Stop);
        }

        let toggle_start = stop_end + BUTTON_GAP_CHARS;
        let toggle_end = toggle_start + char_len(self.toggle);
        if (toggle_start..toggle_end).contains(&offset) {
            return Some(ControlAction::TogglePlayback);
        }

        let next_start = toggle_end + BUTTON_GAP_CHARS;
        let next_end = next_start + char_len(self.next);
        if (next_start..next_end).contains(&offset) {
            return Some(ControlAction::Next);
        }

        None
    }
}

#[derive(Debug, Clone)]
struct RenderTokens {
    icon: String,
    title: String,
    artist: String,
    track: String,
    player: String,
    status: String,
    buttons: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ButtonLayout {
    prefix_chars: usize,
    buttons: PlaybackButtons,
}

pub struct PlaybackModule {
    connection: Option<Connection>,
    state: PlaybackViewState,
    chrome: ModuleChrome,
    format: String,
    icon: String,
    unavailable_icon: String,
    logged_refresh_error: bool,
}

impl PlaybackModule {
    pub fn new(
        format: Option<String>,
        icon: Option<String>,
        unavailable_icon: Option<String>,
        chrome: ModuleChrome,
    ) -> Self {
        Self {
            connection: None,
            state: PlaybackViewState::NoMedia,
            chrome,
            format: resolve_format(format),
            icon: normalize_optional_string(icon).unwrap_or_else(|| DEFAULT_ICON.to_string()),
            unavailable_icon: normalize_optional_string(unavailable_icon)
                .unwrap_or_else(|| DEFAULT_UNAVAILABLE_ICON.to_string()),
            logged_refresh_error: false,
        }
    }

    fn ensure_connection(&mut self) -> Result<Connection, String> {
        if self.connection.is_none() {
            self.connection = Connection::session()
                .map_err(|error| error.to_string())
                .ok();
        }

        self.connection
            .clone()
            .ok_or_else(|| "failed to connect to the session bus".to_string())
    }

    fn render_tokens(&self) -> RenderTokens {
        match &self.state {
            PlaybackViewState::Player(player) => RenderTokens {
                icon: self.icon.clone(),
                title: player.title.clone(),
                artist: player.artist.clone(),
                track: player.track_text(),
                player: player.player_name.clone(),
                status: player.status.label().to_string(),
                buttons: if player.has_visible_controls() {
                    PlaybackButtons::from_status(player.status).text()
                } else {
                    String::new()
                },
            },
            PlaybackViewState::NoMedia => RenderTokens {
                icon: self.unavailable_icon.clone(),
                title: NO_MEDIA_TEXT.to_string(),
                artist: String::new(),
                track: NO_MEDIA_TEXT.to_string(),
                player: String::new(),
                status: "idle".to_string(),
                buttons: String::new(),
            },
            PlaybackViewState::Unavailable => RenderTokens {
                icon: self.unavailable_icon.clone(),
                title: UNAVAILABLE_TEXT.to_string(),
                artist: String::new(),
                track: UNAVAILABLE_TEXT.to_string(),
                player: String::new(),
                status: UNAVAILABLE_TEXT.to_string(),
                buttons: String::new(),
            },
        }
    }

    fn render_text(&self) -> String {
        render_format(&self.format, &self.render_tokens())
    }

    fn button_layout(&self) -> Option<ButtonLayout> {
        let PlaybackViewState::Player(player) = &self.state else {
            return None;
        };
        let (before_buttons, _) = self.format.split_once("{buttons}")?;
        let buttons = PlaybackButtons::from_status(player.status);
        let tokens = self.render_tokens();
        if tokens.buttons.is_empty() {
            return None;
        }

        let prefix = render_format(
            before_buttons,
            &RenderTokens {
                buttons: String::new(),
                ..tokens
            },
        );

        Some(ButtonLayout {
            prefix_chars: char_len(&prefix),
            buttons,
        })
    }

    fn action_for_click(&self, event: &ClickEvent) -> Option<ControlAction> {
        if event.button != MouseButton::Left {
            return None;
        }

        let PlaybackViewState::Player(player) = &self.state else {
            return None;
        };
        let layout = self.button_layout()?;
        let rendered_text = self.render_text();
        let total_chars = char_len(&rendered_text);
        if total_chars == 0 {
            return None;
        }

        let content_width =
            (event.module_width - self.chrome.padding.0 - self.chrome.padding.1).max(0.0);
        if content_width <= 0.0 {
            return None;
        }

        let rel_x = event.x - self.chrome.padding.0;
        if !(0.0..content_width).contains(&rel_x) {
            return None;
        }

        let char_width = content_width / total_chars as f64;
        if char_width <= 0.0 {
            return None;
        }

        let button_start = layout.prefix_chars as f64 * char_width;
        let button_end = button_start + layout.buttons.total_chars() as f64 * char_width;
        if rel_x < button_start || rel_x >= button_end {
            return None;
        }

        let button_offset = ((rel_x - button_start) / char_width)
            .floor()
            .clamp(0.0, layout.buttons.total_chars().saturating_sub(1) as f64)
            as usize;
        let action = layout.buttons.action_for_char_offset(button_offset)?;

        player.supports_action(action).then_some(action)
    }

    fn perform_action(&mut self, action: ControlAction) {
        let PlaybackViewState::Player(player) = &self.state else {
            return;
        };
        if !player.supports_action(action) {
            return;
        }

        let service_name = player.service_name.clone();
        let Ok(connection) = self.ensure_connection() else {
            self.state = PlaybackViewState::Unavailable;
            return;
        };

        let proxy = match MprisPlayerProxyBlocking::builder(&connection)
            .destination(service_name.as_str())
            .and_then(|builder| builder.build())
        {
            Ok(proxy) => proxy,
            Err(error) => {
                self.connection = None;
                log::warn!(
                    "playback module: failed to create MPRIS proxy for '{}': {}",
                    service_name,
                    error
                );
                self.state = PlaybackViewState::Unavailable;
                return;
            }
        };

        let result = match action {
            ControlAction::Previous => proxy.previous(),
            ControlAction::Stop => proxy.stop(),
            ControlAction::TogglePlayback => proxy.play_pause(),
            ControlAction::Next => proxy.next(),
        };

        if let Err(error) = result {
            self.connection = None;
            log::warn!(
                "playback module: failed to send {:?} to '{}': {}",
                action,
                service_name,
                error
            );
        }

        self.update();
    }
}

impl Module for PlaybackModule {
    fn update(&mut self) {
        let connection = match self.ensure_connection() {
            Ok(connection) => connection,
            Err(error) => {
                if !self.logged_refresh_error {
                    log::warn!("playback module: {}", error);
                    self.logged_refresh_error = true;
                }
                self.state = PlaybackViewState::Unavailable;
                return;
            }
        };

        match resolve_playback_state(&connection) {
            Ok(state) => {
                self.state = state;
                self.logged_refresh_error = false;
            }
            Err(error) => {
                self.connection = None;
                if !self.logged_refresh_error {
                    log::warn!(
                        "playback module: failed to refresh playback state: {}",
                        error
                    );
                    self.logged_refresh_error = true;
                }
                self.state = PlaybackViewState::Unavailable;
            }
        }
    }

    fn view(&self) -> ModuleView {
        self.chrome.apply(ModuleView {
            text: self.render_text(),
            style: TextStyle::default(),
            ..Default::default()
        })
    }

    fn click(&mut self, event: ClickEvent) {
        if let Some(action) = self.action_for_click(&event) {
            self.perform_action(action);
        }
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn resolve_format(format: Option<String>) -> String {
    match format {
        Some(format) if !format.trim().is_empty() => format,
        _ => DEFAULT_FORMAT.to_string(),
    }
}

fn render_format(template: &str, tokens: &RenderTokens) -> String {
    let template = if tokens.buttons.is_empty() {
        template
            .replace(" {buttons}", "")
            .replace("{buttons} ", "")
            .replace("{buttons}", "")
    } else {
        template.replace("{buttons}", &tokens.buttons)
    };

    template
        .replace("{icon}", &tokens.icon)
        .replace("{title}", &tokens.title)
        .replace("{artist}", &tokens.artist)
        .replace("{track}", &tokens.track)
        .replace("{player}", &tokens.player)
        .replace("{status}", &tokens.status)
}

fn resolve_playback_state(connection: &Connection) -> Result<PlaybackViewState, String> {
    let service_names = list_mpris_service_names(connection)?;
    if service_names.is_empty() {
        return Ok(PlaybackViewState::NoMedia);
    }

    let mut best_player: Option<PlayerState> = None;

    for service_name in service_names {
        let player = match query_player_state(connection, &service_name) {
            Ok(player) => player,
            Err(error) => {
                log::debug!(
                    "playback module: failed to query '{}': {}",
                    service_name,
                    error
                );
                continue;
            }
        };

        if should_prefer_player(best_player.as_ref(), &player) {
            best_player = Some(player);
        }
    }

    match best_player {
        Some(player) => Ok(PlaybackViewState::Player(player)),
        None => Err("found MPRIS services, but none could be queried successfully".to_string()),
    }
}

fn list_mpris_service_names(connection: &Connection) -> Result<Vec<String>, String> {
    let proxy = Proxy::new(connection, DBUS_DESTINATION, DBUS_PATH, DBUS_INTERFACE)
        .map_err(|error| error.to_string())?;
    let mut names: Vec<String> = proxy
        .call("ListNames", &())
        .map_err(|error| error.to_string())?;
    names.retain(|name| name.starts_with(MPRIS_PREFIX));
    names.sort_unstable();
    Ok(names)
}

fn query_player_state(connection: &Connection, service_name: &str) -> Result<PlayerState, String> {
    let root_proxy = MprisRootProxyBlocking::builder(connection)
        .destination(service_name)
        .and_then(|builder| builder.build())
        .map_err(|error| error.to_string())?;
    let player_proxy = MprisPlayerProxyBlocking::builder(connection)
        .destination(service_name)
        .and_then(|builder| builder.build())
        .map_err(|error| error.to_string())?;

    let metadata = player_proxy.metadata().map_err(|error| error.to_string())?;

    Ok(PlayerState {
        service_name: service_name.to_string(),
        player_name: root_proxy
            .identity()
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| fallback_player_name(service_name)),
        title: metadata_text(&metadata, &["xesam:title", "xesam:url"]),
        artist: metadata_list(&metadata, "xesam:artist").join(", "),
        status: PlaybackStatus::from_raw(
            &player_proxy
                .playback_status()
                .map_err(|error| error.to_string())?,
        ),
        can_control: player_proxy.can_control().unwrap_or(false),
        can_go_previous: player_proxy.can_go_previous().unwrap_or(false),
        can_go_next: player_proxy.can_go_next().unwrap_or(false),
        can_toggle_playback: player_proxy.can_play().unwrap_or(false)
            || player_proxy.can_pause().unwrap_or(false),
    })
}

fn fallback_player_name(service_name: &str) -> String {
    service_name
        .strip_prefix(MPRIS_PREFIX)
        .unwrap_or(service_name)
        .to_string()
}

fn metadata_text(metadata: &HashMap<String, OwnedValue>, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| metadata.get(*key))
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| String::try_from(value).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

fn metadata_list(metadata: &HashMap<String, OwnedValue>, key: &str) -> Vec<String> {
    metadata
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| Vec::<String>::try_from(value).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn should_prefer_player(current: Option<&PlayerState>, candidate: &PlayerState) -> bool {
    let Some(current) = current else {
        return true;
    };

    if candidate.status.rank() != current.status.rank() {
        return candidate.status.rank() > current.status.rank();
    }

    let current_has_track = !current.track_text().trim().is_empty();
    let candidate_has_track = !candidate.track_text().trim().is_empty();
    if candidate_has_track != current_has_track {
        return candidate_has_track;
    }

    candidate.player_name < current.player_name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::module::ModuleChrome;

    fn default_chrome() -> ModuleChrome {
        ModuleChrome {
            foreground: None,
            background: None,
            padding: (8.0, 8.0),
            icon_spacing: None,
        }
    }

    fn player_state() -> PlayerState {
        PlayerState {
            service_name: "org.mpris.MediaPlayer2.spotify".to_string(),
            player_name: "Spotify".to_string(),
            title: "The Chain".to_string(),
            artist: "Fleetwood Mac".to_string(),
            status: PlaybackStatus::Playing,
            can_control: true,
            can_go_previous: true,
            can_go_next: true,
            can_toggle_playback: true,
        }
    }

    fn module_with_player(format: Option<String>) -> PlaybackModule {
        let mut module = PlaybackModule::new(format, None, None, default_chrome());
        module.state = PlaybackViewState::Player(player_state());
        module
    }

    fn module_width(module: &PlaybackModule) -> f64 {
        module.chrome.padding.0 + module.chrome.padding.1 + char_len(&module.view().text) as f64
    }

    fn click_event(module: &PlaybackModule, x: f64) -> ClickEvent {
        ClickEvent {
            x,
            screen_x: x,
            module_width: module_width(module),
            y: 0.0,
            screen_y: 0.0,
            button: MouseButton::Left,
        }
    }

    fn button_click_x(module: &PlaybackModule, action: ControlAction) -> f64 {
        let layout = module.button_layout().expect("buttons should be rendered");
        let previous_chars = char_len(layout.buttons.previous);
        let stop_chars = char_len(layout.buttons.stop);
        let toggle_chars = char_len(layout.buttons.toggle);
        let next_chars = char_len(layout.buttons.next);

        let offset = match action {
            ControlAction::Previous => layout.prefix_chars as f64 + previous_chars as f64 / 2.0,
            ControlAction::Stop => {
                layout.prefix_chars as f64
                    + previous_chars as f64
                    + BUTTON_GAP_CHARS as f64
                    + stop_chars as f64 / 2.0
            }
            ControlAction::TogglePlayback => {
                layout.prefix_chars as f64
                    + previous_chars as f64
                    + BUTTON_GAP_CHARS as f64
                    + stop_chars as f64
                    + BUTTON_GAP_CHARS as f64
                    + toggle_chars as f64 / 2.0
            }
            ControlAction::Next => {
                layout.prefix_chars as f64
                    + previous_chars as f64
                    + BUTTON_GAP_CHARS as f64
                    + stop_chars as f64
                    + BUTTON_GAP_CHARS as f64
                    + toggle_chars as f64
                    + BUTTON_GAP_CHARS as f64
                    + next_chars as f64 / 2.0
            }
        };

        module.chrome.padding.0 + offset
    }

    #[test]
    fn uses_default_icon() {
        let module = module_with_player(None);

        assert!(module.view().text.starts_with(" "));
    }

    #[test]
    fn uses_configured_icon() {
        let mut module = PlaybackModule::new(None, Some("NOW".to_string()), None, default_chrome());
        module.state = PlaybackViewState::Player(player_state());

        assert!(module.view().text.starts_with("NOW "));
    }

    #[test]
    fn renders_track_and_buttons() {
        let module = module_with_player(None);

        assert_eq!(module.view().text, " Fleetwood Mac - The Chain 󰒮 󰓛 󰏤 󰒭");
    }

    #[test]
    fn renders_custom_format_tokens() {
        let module = module_with_player(Some("{player}: {title} [{status}] {buttons}".to_string()));

        assert_eq!(module.view().text, "Spotify: The Chain [playing] 󰒮 󰓛 󰏤 󰒭");
    }

    #[test]
    fn strips_button_gap_when_buttons_are_absent() {
        let module = PlaybackModule::new(None, None, None, default_chrome());

        assert_eq!(module.view().text, "󰎈 no media");
    }

    #[test]
    fn keeps_user_format_whitespace() {
        let module = module_with_player(Some("  {track} {buttons}  ".to_string()));

        assert_eq!(module.view().text, "  Fleetwood Mac - The Chain 󰒮 󰓛 󰏤 󰒭  ");
    }

    #[test]
    fn prefers_playing_player_over_paused_player() {
        let paused = PlayerState {
            status: PlaybackStatus::Paused,
            ..player_state()
        };
        let playing = PlayerState {
            player_name: "VLC".to_string(),
            ..player_state()
        };

        assert!(should_prefer_player(Some(&paused), &playing));
        assert!(!should_prefer_player(Some(&playing), &paused));
    }

    #[test]
    fn click_mapping_uses_button_region_in_default_format() {
        let module = module_with_player(None);

        assert_eq!(
            module.action_for_click(&click_event(
                &module,
                button_click_x(&module, ControlAction::Previous)
            )),
            Some(ControlAction::Previous)
        );
        assert_eq!(
            module.action_for_click(&click_event(
                &module,
                button_click_x(&module, ControlAction::Stop)
            )),
            Some(ControlAction::Stop)
        );
        assert_eq!(
            module.action_for_click(&click_event(
                &module,
                button_click_x(&module, ControlAction::TogglePlayback)
            )),
            Some(ControlAction::TogglePlayback)
        );
        assert_eq!(
            module.action_for_click(&click_event(
                &module,
                button_click_x(&module, ControlAction::Next)
            )),
            Some(ControlAction::Next)
        );
    }

    #[test]
    fn click_mapping_tracks_custom_button_position() {
        let module = module_with_player(Some("{buttons} {track}".to_string()));

        assert_eq!(
            module.action_for_click(&click_event(
                &module,
                button_click_x(&module, ControlAction::Stop)
            )),
            Some(ControlAction::Stop)
        );
    }
}
