//! Quick-settings panel: the dropdown drawn by the `panel` module.
//!
//! The main page has up to three sections, top to bottom:
//! - a row of round-cornered icon buttons (some left, some right aligned),
//! - a media card (what is playing, previous/play/next, a seek bar),
//! - full-width sliders (icon, track, optional arrow button),
//! - a two-column grid of pill-shaped toggle tiles (optional arrow segment).
//!
//! An arrow can open a sub-page in place of the main page: a back button and
//! title, then a list of section headers, selectable items, labelled sliders
//! (a mixer) and notes.
//!
//! This file is pure geometry and drawing. The module that owns the panel
//! decides what is in it and what clicks do.

use std::sync::Arc;

use crate::core::module::IconData;
use crate::renderer::color::Color;
use crate::renderer::primitives::{Point, Rect, Renderer, TextStyle};

const PADDING: f64 = 16.0;
const SECTION_GAP: f64 = 16.0;
const BORDER: f64 = 1.0;

const BUTTON_SIZE: f64 = 40.0;
const BUTTON_GAP: f64 = 8.0;
const BUTTON_RADIUS: f64 = 12.0;

const SLIDER_ROW_HEIGHT: f64 = 32.0;
const SLIDER_ROW_GAP: f64 = 8.0;
const SLIDER_ICON_WIDTH: f64 = 32.0;
const SLIDER_TRACK_HEIGHT: f64 = 6.0;
const SLIDER_KNOB_RADIUS: f64 = 8.0;
const SLIDER_MENU_GAP: f64 = 10.0;

const TILE_HEIGHT: f64 = 50.0;
const TILE_GAP: f64 = 10.0;
const TILE_ARROW_WIDTH: f64 = 34.0;
/// Horizontal centre of a tile's icon, from the tile's left edge.
const TILE_ICON_CENTER: f64 = 22.0;
const TILE_LABEL_X: f64 = 40.0;
const TILE_TEXT_RIGHT_PAD: f64 = 8.0;

const MEDIA_HEIGHT: f64 = 82.0;
const MEDIA_PAD: f64 = 12.0;
const MEDIA_RADIUS: f64 = 16.0;
const MEDIA_ART: f64 = 40.0;
const MEDIA_BUTTON: f64 = 32.0;
const MEDIA_BUTTON_GAP: f64 = 4.0;
const MEDIA_TRACK_HEIGHT: f64 = 4.0;
/// Space between the title row and the seek bar.
const MEDIA_TRACK_GAP: f64 = 12.0;

const PAGE_HEADER_HEIGHT: f64 = 34.0;
const PAGE_HEADER_GAP: f64 = 12.0;
const PAGE_ROW_GAP: f64 = 4.0;
const PAGE_SECTION_HEIGHT: f64 = 26.0;
/// Extra space above every section header except the first row.
const PAGE_SECTION_SPACING: f64 = 8.0;
const PAGE_ITEM_HEIGHT: f64 = 40.0;
const PAGE_ITEM_RADIUS: f64 = 12.0;
const PAGE_SLIDER_LABEL_HEIGHT: f64 = 22.0;
const PAGE_SLIDER_ROW_HEIGHT: f64 = 30.0;
const PAGE_NOTE_HEIGHT: f64 = 32.0;
/// Width of the icon column on page rows.
const PAGE_ICON_WIDTH: f64 = 32.0;
const PAGE_TEXT_X: f64 = 40.0;
const PAGE_INNER_PAD: f64 = 10.0;

/// nf-md-chevron_right
const ARROW_GLYPH: &str = "\u{f0142}";
/// nf-md-chevron_left
const BACK_GLYPH: &str = "\u{f0141}";
/// nf-md-check
const CHECK_GLYPH: &str = "\u{f012c}";
/// nf-md-skip_previous, play, pause, skip_next
const PREVIOUS_GLYPH: &str = "\u{f04ae}";
const PLAY_GLYPH: &str = "\u{f040a}";
const PAUSE_GLYPH: &str = "\u{f03e4}";
const NEXT_GLYPH: &str = "\u{f04ad}";

#[derive(Debug, Clone, PartialEq)]
pub struct PanelStyle {
    pub width: f64,
    pub corner_radius: f64,
    pub background: Color,
    pub border: Color,
    /// Fill of buttons, inactive tiles and slider tracks.
    pub surface: Color,
    /// Fill of active tiles and the filled part of sliders.
    pub accent: Color,
    /// Text and icons drawn on top of `accent`. Other text uses the bar's
    /// foreground color.
    pub on_accent: Color,
    /// Fill of a button waiting for its confirmation click.
    pub danger: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelButton {
    pub icon: String,
    pub align_right: bool,
    /// The button asked for confirmation and the next click runs it.
    pub armed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelSlider {
    pub icon: String,
    /// Current value, 0.0 to 1.0.
    pub value: f64,
    pub has_menu: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelTile {
    pub icon: String,
    pub label: String,
    pub subtitle: Option<String>,
    pub active: bool,
    pub has_menu: bool,
}

/// The media card: the player that is playing (or was last).
#[derive(Debug, Clone, PartialEq)]
pub struct PanelMedia {
    /// Shown on the card's artwork square when there is no artwork, e.g.
    /// the player's logo.
    pub icon: String,
    /// The track's artwork, already square.
    pub art: Option<Arc<IconData>>,
    pub title: String,
    pub artist: Option<String>,
    pub playing: bool,
    /// How far into the track, 0.0 to 1.0, when the player says.
    pub progress: Option<f64>,
}

/// One row of a sub-page.
#[derive(Debug, Clone, PartialEq)]
pub enum PageRow {
    /// Small heading above a group of rows.
    Section(String),
    /// A clickable entry, e.g. a network or a device.
    Item {
        icon: String,
        label: String,
        /// Dim text on the right, e.g. "Connected".
        detail: Option<String>,
        /// Draws a check mark (the current choice).
        checked: bool,
    },
    /// A labelled slider with a mute button, e.g. one entry of a mixer.
    Slider {
        icon: String,
        label: String,
        /// 0.0 to 1.0.
        value: f64,
        muted: bool,
        /// `Some` makes the label line clickable (to choose this entry) and
        /// `Some(true)` draws a check mark on it.
        checked: Option<bool>,
    },
    /// Dim informational text, e.g. "No apps are playing audio".
    Note(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelPage {
    pub title: String,
    pub rows: Vec<PageRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelView {
    pub buttons: Vec<PanelButton>,
    /// What is playing, if anything (the card is hidden otherwise).
    pub media: Option<PanelMedia>,
    pub sliders: Vec<PanelSlider>,
    pub tiles: Vec<PanelTile>,
    /// When set, this sub-page is shown instead of the main page.
    pub page: Option<PanelPage>,
    pub style: PanelStyle,
}

/// A clickable element of the panel. Indices refer to the vectors in
/// [`PanelView`] (or to `PanelPage::rows` for the page targets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelTarget {
    Button(usize),
    MediaPrevious,
    MediaPlayPause,
    MediaNext,
    /// The media card's seek bar.
    MediaSeek,
    Slider(usize),
    SliderMenu(usize),
    Tile(usize),
    TileMenu(usize),
    /// The sub-page's back button.
    Back,
    /// A page item, or the label line of a selectable page slider.
    PageItem(usize),
    PageSlider(usize),
    /// The mute button of a page slider.
    PageSliderIcon(usize),
}

impl PanelTarget {
    pub fn is_slider(self) -> bool {
        matches!(
            self,
            PanelTarget::Slider(_) | PanelTarget::PageSlider(_) | PanelTarget::MediaSeek
        )
    }
}

#[derive(Debug, Clone)]
pub struct PanelLayout {
    pub width: f64,
    pub height: f64,
    /// Every clickable element with its hit rectangle. Hover and hit indices
    /// refer to positions in this list.
    pub targets: Vec<(PanelTarget, Rect)>,
    /// The visible track of every slider.
    tracks: Vec<(PanelTarget, Rect)>,
    /// The area of each page row (empty on the main page).
    page_rows: Vec<Rect>,
    /// The media card's background, when shown.
    media_card: Option<Rect>,
}

impl PanelLayout {
    /// The element under `(x, y)`: its index in `targets`, what it is, and
    /// for sliders the position along the track (0.0 to 1.0).
    pub fn hit_test(&self, x: f64, y: f64) -> Option<(usize, PanelTarget, f64)> {
        self.targets
            .iter()
            .position(|(_, rect)| contains(rect, x, y))
            .map(|index| {
                let target = self.targets[index].0;
                (index, target, self.fraction(target, x))
            })
    }

    /// Re-evaluate the target at `index` for pointer position `x`, ignoring
    /// the vertical position. Used while dragging a slider, so the drag keeps
    /// working when the pointer leaves the track.
    pub fn drag(&self, index: usize, x: f64) -> Option<(PanelTarget, f64)> {
        let (target, _) = self.targets.get(index)?;
        Some((*target, self.fraction(*target, x)))
    }

    fn fraction(&self, target: PanelTarget, x: f64) -> f64 {
        self.track_of(target)
            .filter(|track| track.width > 0.0)
            .map(|track| {
                // The seek bar has no knob at rest, so it maps edge to edge.
                let (start, travel) = if target == PanelTarget::MediaSeek {
                    (track.x, track.width)
                } else {
                    knob_travel(&track)
                };
                ((x - start) / travel).clamp(0.0, 1.0)
            })
            .unwrap_or(0.0)
    }

    fn track_of(&self, target: PanelTarget) -> Option<Rect> {
        self.tracks
            .iter()
            .find(|(t, _)| *t == target)
            .map(|(_, rect)| *rect)
    }

    fn rect_of(&self, target: PanelTarget) -> Option<Rect> {
        self.targets
            .iter()
            .find(|(t, _)| *t == target)
            .map(|(_, rect)| *rect)
    }
}

/// Where the knob centre can go on `track`: (leftmost position, distance to
/// the rightmost). The knob stays fully inside the track at both ends, so a
/// full slider ends exactly where an empty one does.
fn knob_travel(track: &Rect) -> (f64, f64) {
    let start = track.x + SLIDER_KNOB_RADIUS;
    (start, (track.width - 2.0 * SLIDER_KNOB_RADIUS).max(1.0))
}

fn contains(rect: &Rect, x: f64, y: f64) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

pub fn layout_panel(view: &PanelView) -> PanelLayout {
    let width = view.style.width.max(2.0 * PADDING + 120.0).round();
    match &view.page {
        Some(page) => layout_page(page, width),
        None => layout_main(view, width),
    }
}

fn layout_main(view: &PanelView, width: f64) -> PanelLayout {
    let inner_left = PADDING;
    let inner_right = width - PADDING;
    let mut targets = Vec::new();
    let mut tracks = Vec::with_capacity(view.sliders.len());
    let mut y = PADDING;
    let mut any_section = false;

    // Buttons: left group grows rightwards, right group is flush right and
    // keeps config order.
    if !view.buttons.is_empty() {
        let right_count = view.buttons.iter().filter(|b| b.align_right).count() as f64;
        let right_width = right_count * BUTTON_SIZE + (right_count - 1.0).max(0.0) * BUTTON_GAP;
        let mut left_x = inner_left;
        let mut right_x = inner_right - right_width;
        for (i, button) in view.buttons.iter().enumerate() {
            let x = if button.align_right {
                let x = right_x;
                right_x += BUTTON_SIZE + BUTTON_GAP;
                x
            } else {
                let x = left_x;
                left_x += BUTTON_SIZE + BUTTON_GAP;
                x
            };
            targets.push((
                PanelTarget::Button(i),
                Rect {
                    x,
                    y,
                    width: BUTTON_SIZE,
                    height: BUTTON_SIZE,
                },
            ));
        }
        y += BUTTON_SIZE + SECTION_GAP;
        any_section = true;
    }

    let mut media_card = None;
    if let Some(media) = &view.media {
        let card = Rect {
            x: inner_left,
            y,
            width: inner_right - inner_left,
            height: MEDIA_HEIGHT,
        };
        let row_y = y + MEDIA_PAD;
        let button_y = row_y + (MEDIA_ART - MEDIA_BUTTON) / 2.0;
        let next_x = card.x + card.width - MEDIA_PAD - MEDIA_BUTTON;
        let buttons = [
            (
                PanelTarget::MediaPrevious,
                next_x - 2.0 * (MEDIA_BUTTON + MEDIA_BUTTON_GAP),
            ),
            (
                PanelTarget::MediaPlayPause,
                next_x - (MEDIA_BUTTON + MEDIA_BUTTON_GAP),
            ),
            (PanelTarget::MediaNext, next_x),
        ];
        for (target, x) in buttons {
            targets.push((
                target,
                Rect {
                    x,
                    y: button_y,
                    width: MEDIA_BUTTON,
                    height: MEDIA_BUTTON,
                },
            ));
        }
        if media.progress.is_some() {
            let track = Rect {
                x: card.x + MEDIA_PAD,
                y: row_y + MEDIA_ART + MEDIA_TRACK_GAP,
                width: card.width - 2.0 * MEDIA_PAD,
                height: MEDIA_TRACK_HEIGHT,
            };
            // Generous hit area: the bar itself is only 4px tall.
            targets.push((
                PanelTarget::MediaSeek,
                Rect {
                    y: track.y - 7.0,
                    height: MEDIA_TRACK_HEIGHT + 14.0,
                    ..track
                },
            ));
            tracks.push((PanelTarget::MediaSeek, track));
        }
        media_card = Some(card);
        y += MEDIA_HEIGHT + SECTION_GAP;
        any_section = true;
    }

    if !view.sliders.is_empty() {
        for (i, slider) in view.sliders.iter().enumerate() {
            let menu_width = if slider.has_menu {
                SLIDER_ROW_HEIGHT + SLIDER_MENU_GAP
            } else {
                0.0
            };
            let track_x = inner_left + SLIDER_ICON_WIDTH;
            let track_right = inner_right - menu_width;
            let track = Rect {
                x: track_x,
                y: y + (SLIDER_ROW_HEIGHT - SLIDER_TRACK_HEIGHT) / 2.0,
                width: (track_right - track_x).max(1.0),
                height: SLIDER_TRACK_HEIGHT,
            };
            // The hit area covers the full row height so the slider is easy
            // to grab.
            targets.push((
                PanelTarget::Slider(i),
                Rect {
                    x: track.x,
                    y,
                    width: track.width,
                    height: SLIDER_ROW_HEIGHT,
                },
            ));
            if slider.has_menu {
                targets.push((
                    PanelTarget::SliderMenu(i),
                    Rect {
                        x: inner_right - SLIDER_ROW_HEIGHT,
                        y,
                        width: SLIDER_ROW_HEIGHT,
                        height: SLIDER_ROW_HEIGHT,
                    },
                ));
            }
            tracks.push((PanelTarget::Slider(i), track));
            y += SLIDER_ROW_HEIGHT + SLIDER_ROW_GAP;
        }
        y += SECTION_GAP - SLIDER_ROW_GAP;
        any_section = true;
    }

    if !view.tiles.is_empty() {
        let column_width = ((inner_right - inner_left - TILE_GAP) / 2.0).floor();
        for (i, tile) in view.tiles.iter().enumerate() {
            let x = if i % 2 == 0 {
                inner_left
            } else {
                inner_right - column_width
            };
            let tile_y = y + (i / 2) as f64 * (TILE_HEIGHT + TILE_GAP);
            let main_width = if tile.has_menu {
                column_width - TILE_ARROW_WIDTH
            } else {
                column_width
            };
            targets.push((
                PanelTarget::Tile(i),
                Rect {
                    x,
                    y: tile_y,
                    width: main_width,
                    height: TILE_HEIGHT,
                },
            ));
            if tile.has_menu {
                targets.push((
                    PanelTarget::TileMenu(i),
                    Rect {
                        x: x + main_width,
                        y: tile_y,
                        width: TILE_ARROW_WIDTH,
                        height: TILE_HEIGHT,
                    },
                ));
            }
        }
        let rows = view.tiles.len().div_ceil(2) as f64;
        y += rows * TILE_HEIGHT + (rows - 1.0) * TILE_GAP + SECTION_GAP;
        any_section = true;
    }

    let height = if any_section {
        y - SECTION_GAP + PADDING
    } else {
        2.0 * PADDING
    };

    PanelLayout {
        width,
        height: height.ceil(),
        targets,
        tracks,
        page_rows: Vec::new(),
        media_card,
    }
}

fn layout_page(page: &PanelPage, width: f64) -> PanelLayout {
    let inner_left = PADDING;
    let inner_width = width - 2.0 * PADDING;
    let mut targets = vec![(
        PanelTarget::Back,
        Rect {
            x: inner_left,
            y: PADDING,
            width: PAGE_HEADER_HEIGHT,
            height: PAGE_HEADER_HEIGHT,
        },
    )];
    let mut tracks = Vec::new();
    let mut page_rows = Vec::with_capacity(page.rows.len());
    let mut y = PADDING + PAGE_HEADER_HEIGHT + PAGE_HEADER_GAP;

    for (i, row) in page.rows.iter().enumerate() {
        if matches!(row, PageRow::Section(_)) && i > 0 {
            y += PAGE_SECTION_SPACING;
        }
        let height = match row {
            PageRow::Section(_) => PAGE_SECTION_HEIGHT,
            PageRow::Item { .. } => PAGE_ITEM_HEIGHT,
            PageRow::Slider { .. } => PAGE_SLIDER_LABEL_HEIGHT + PAGE_SLIDER_ROW_HEIGHT,
            PageRow::Note(_) => PAGE_NOTE_HEIGHT,
        };
        let rect = Rect {
            x: inner_left,
            y,
            width: inner_width,
            height,
        };
        match row {
            PageRow::Item { .. } => targets.push((PanelTarget::PageItem(i), rect)),
            PageRow::Slider { checked, .. } => {
                if checked.is_some() {
                    targets.push((
                        PanelTarget::PageItem(i),
                        Rect {
                            height: PAGE_SLIDER_LABEL_HEIGHT,
                            ..rect
                        },
                    ));
                }
                let row_y = y + PAGE_SLIDER_LABEL_HEIGHT;
                targets.push((
                    PanelTarget::PageSliderIcon(i),
                    Rect {
                        x: inner_left,
                        y: row_y,
                        width: PAGE_ICON_WIDTH,
                        height: PAGE_SLIDER_ROW_HEIGHT,
                    },
                ));
                let track = Rect {
                    x: inner_left + PAGE_TEXT_X,
                    y: row_y + (PAGE_SLIDER_ROW_HEIGHT - SLIDER_TRACK_HEIGHT) / 2.0,
                    width: (inner_width - PAGE_TEXT_X).max(1.0),
                    height: SLIDER_TRACK_HEIGHT,
                };
                targets.push((
                    PanelTarget::PageSlider(i),
                    Rect {
                        x: track.x,
                        y: row_y,
                        width: track.width,
                        height: PAGE_SLIDER_ROW_HEIGHT,
                    },
                ));
                tracks.push((PanelTarget::PageSlider(i), track));
            }
            PageRow::Section(_) | PageRow::Note(_) => {}
        }
        page_rows.push(rect);
        y += height + PAGE_ROW_GAP;
    }

    let height = y - PAGE_ROW_GAP + PADDING;
    PanelLayout {
        width,
        height: height.ceil(),
        targets,
        tracks,
        page_rows,
        media_card: None,
    }
}

struct Palette<'a> {
    style: &'a PanelStyle,
    text: Color,
    dim: Color,
    label: TextStyle,
    plain: TextStyle,
    icon: TextStyle,
    small: TextStyle,
}

pub fn draw_panel<R: Renderer>(
    renderer: &mut R,
    view: &PanelView,
    layout: &PanelLayout,
    text_style: &TextStyle,
    hovered: Option<usize>,
) {
    let style = &view.style;
    let full = Rect {
        x: 0.0,
        y: 0.0,
        width: layout.width,
        height: layout.height,
    };
    renderer.draw_rounded_rect(full, style.corner_radius, style.border);
    renderer.draw_rounded_rect(
        inset(full, BORDER),
        (style.corner_radius - BORDER).max(0.0),
        style.background,
    );

    let text = text_style.color;
    let palette = Palette {
        style,
        text,
        dim: text.with_alpha(text.a * 0.6),
        label: text_style.clone(),
        plain: TextStyle {
            bold: false,
            ..text_style.clone()
        },
        icon: TextStyle {
            font_size: text_style.font_size * 1.2,
            bold: false,
            ..text_style.clone()
        },
        small: TextStyle {
            font_size: text_style.font_size * 0.82,
            bold: false,
            ..text_style.clone()
        },
    };
    let hovered_target = hovered.and_then(|i| layout.targets.get(i)).map(|(t, _)| *t);

    match &view.page {
        Some(page) => draw_page(renderer, page, layout, &palette, hovered_target),
        None => draw_main(renderer, view, layout, &palette, hovered_target),
    }
}

fn draw_main<R: Renderer>(
    renderer: &mut R,
    view: &PanelView,
    layout: &PanelLayout,
    p: &Palette,
    hovered: Option<PanelTarget>,
) {
    let style = p.style;
    let is_hovered = |target: PanelTarget| hovered == Some(target);
    let tile_icon = TextStyle {
        font_size: p.label.font_size * 1.4,
        ..p.icon.clone()
    };

    for (i, button) in view.buttons.iter().enumerate() {
        let target = PanelTarget::Button(i);
        let Some(rect) = layout.rect_of(target) else {
            continue;
        };
        let (fill, fg) = if button.armed {
            (style.danger, style.on_accent)
        } else {
            (style.surface, p.text)
        };
        renderer.draw_rounded_rect(rect, BUTTON_RADIUS, hover_color(fill, is_hovered(target)));
        draw_centered(renderer, rect, &button.icon, &with_color(&p.icon, fg));
    }

    if let (Some(media), Some(card)) = (&view.media, layout.media_card) {
        draw_media(renderer, media, card, layout, p, hovered);
    }

    for (i, slider) in view.sliders.iter().enumerate() {
        let Some(track) = layout.track_of(PanelTarget::Slider(i)) else {
            continue;
        };
        let row_y = track.y - (SLIDER_ROW_HEIGHT - SLIDER_TRACK_HEIGHT) / 2.0;
        draw_centered(
            renderer,
            Rect {
                x: PADDING,
                y: row_y,
                width: SLIDER_ICON_WIDTH - 8.0,
                height: SLIDER_ROW_HEIGHT,
            },
            &slider.icon,
            &p.icon,
        );
        draw_track(
            renderer,
            track,
            slider.value,
            style.accent,
            style.surface,
            is_hovered(PanelTarget::Slider(i)),
        );

        if let Some(rect) = layout.rect_of(PanelTarget::SliderMenu(i)) {
            let hovered = is_hovered(PanelTarget::SliderMenu(i));
            renderer.draw_rounded_rect(
                rect,
                rect.height / 2.0,
                hover_color(style.surface, hovered),
            );
            draw_centered(renderer, rect, ARROW_GLYPH, &p.icon);
        }
    }

    for (i, tile) in view.tiles.iter().enumerate() {
        let Some(main) = layout.rect_of(PanelTarget::Tile(i)) else {
            continue;
        };
        let arrow = layout.rect_of(PanelTarget::TileMenu(i));
        let whole = Rect {
            width: main.width + arrow.map_or(0.0, |a| a.width),
            ..main
        };
        let radius = TILE_HEIGHT / 2.0;
        let (fill, fg) = if tile.active {
            (style.accent, style.on_accent)
        } else {
            (style.surface, p.text)
        };

        renderer.draw_rounded_rect(
            whole,
            radius,
            hover_color(fill, is_hovered(PanelTarget::Tile(i))),
        );
        if let Some(arrow) = arrow {
            // Same pill clipped to the arrow segment, in a lighter shade, so
            // the segment keeps the pill's rounded right end.
            let shade = fill.mix(Color::WHITE, 0.14);
            renderer.push_clip(arrow);
            renderer.draw_rounded_rect(
                whole,
                radius,
                hover_color(shade, is_hovered(PanelTarget::TileMenu(i))),
            );
            renderer.pop_clip();
            draw_centered(renderer, arrow, ARROW_GLYPH, &with_color(&p.icon, fg));
        }

        draw_centered(
            renderer,
            Rect {
                x: main.x + TILE_ICON_CENTER - 16.0,
                y: main.y,
                width: 32.0,
                height: main.height,
            },
            &tile.icon,
            &with_color(&tile_icon, fg),
        );

        let text_x = main.x + TILE_LABEL_X;
        let text_width = (main.x + main.width - TILE_TEXT_RIGHT_PAD - text_x).max(0.0);
        let label_style = TextStyle {
            font_size: p.label.font_size * 0.92,
            ..with_color(&p.label, fg)
        };
        let sub_style = with_color(&p.small, fg.with_alpha(fg.a * 0.75));
        draw_two_lines(
            renderer,
            Rect {
                x: text_x,
                width: text_width,
                ..main
            },
            &tile.label,
            &label_style,
            tile.subtitle.as_deref(),
            &sub_style,
        );
    }
}

fn draw_media<R: Renderer>(
    renderer: &mut R,
    media: &PanelMedia,
    card: Rect,
    layout: &PanelLayout,
    p: &Palette,
    hovered: Option<PanelTarget>,
) {
    let style = p.style;
    let is_hovered = |target: PanelTarget| hovered == Some(target);
    renderer.draw_rounded_rect(card, MEDIA_RADIUS, style.surface);

    let art = Rect {
        x: card.x + MEDIA_PAD,
        y: card.y + MEDIA_PAD,
        width: MEDIA_ART,
        height: MEDIA_ART,
    };
    match &media.art {
        Some(image) => {
            renderer.push_rounded_clip(art, 10.0);
            renderer.draw_icon(
                Point { x: art.x, y: art.y },
                &image.pixels,
                image.width,
                image.height,
                MEDIA_ART as u32,
            );
            renderer.pop_clip();
        }
        None => {
            renderer.draw_rounded_rect(art, 10.0, style.accent);
            let art_icon = TextStyle {
                font_size: p.label.font_size * 1.5,
                ..with_color(&p.icon, style.on_accent)
            };
            draw_centered(renderer, art, &media.icon, &art_icon);
        }
    }

    let controls_x = layout
        .rect_of(PanelTarget::MediaPrevious)
        .map_or(card.x + card.width, |r| r.x);
    let text_x = art.x + art.width + 10.0;
    draw_two_lines(
        renderer,
        Rect {
            x: text_x,
            width: (controls_x - 8.0 - text_x).max(0.0),
            ..art
        },
        &media.title,
        &TextStyle {
            font_size: p.label.font_size * 0.92,
            ..with_color(&p.label, p.text)
        },
        media.artist.as_deref(),
        &with_color(&p.small, p.dim),
    );

    let play = if media.playing {
        PAUSE_GLYPH
    } else {
        PLAY_GLYPH
    };
    for (target, glyph) in [
        (PanelTarget::MediaPrevious, PREVIOUS_GLYPH),
        (PanelTarget::MediaPlayPause, play),
        (PanelTarget::MediaNext, NEXT_GLYPH),
    ] {
        let Some(rect) = layout.rect_of(target) else {
            continue;
        };
        // Play/pause is the main action: a filled accent circle.
        let (fill, fg) = if target == PanelTarget::MediaPlayPause {
            (Some(style.accent), style.on_accent)
        } else if is_hovered(target) {
            (Some(style.surface.mix(Color::WHITE, 0.10)), p.text)
        } else {
            (None, p.text)
        };
        if let Some(fill) = fill {
            renderer.draw_rounded_rect(
                rect,
                rect.height / 2.0,
                hover_color(
                    fill,
                    is_hovered(target) && target == PanelTarget::MediaPlayPause,
                ),
            );
        }
        draw_centered(renderer, rect, glyph, &with_color(&p.icon, fg));
    }

    if let (Some(progress), Some(track)) = (media.progress, layout.track_of(PanelTarget::MediaSeek))
    {
        let radius = track.height / 2.0;
        renderer.draw_rounded_rect(track, radius, p.text.with_alpha(0.15));
        let filled = track.width * progress.clamp(0.0, 1.0);
        renderer.draw_rounded_rect(
            Rect {
                width: filled,
                ..track
            },
            radius,
            style.accent,
        );
        if is_hovered(PanelTarget::MediaSeek) {
            let r = 6.0;
            renderer.draw_rounded_rect(
                Rect {
                    x: track.x + filled - r,
                    y: track.y + radius - r,
                    width: 2.0 * r,
                    height: 2.0 * r,
                },
                r,
                style.accent,
            );
        }
    }
}

fn draw_page<R: Renderer>(
    renderer: &mut R,
    page: &PanelPage,
    layout: &PanelLayout,
    p: &Palette,
    hovered: Option<PanelTarget>,
) {
    let style = p.style;
    let is_hovered = |target: PanelTarget| hovered == Some(target);

    if let Some(back) = layout.rect_of(PanelTarget::Back) {
        renderer.draw_rounded_rect(
            back,
            back.height / 2.0,
            hover_color(style.surface, is_hovered(PanelTarget::Back)),
        );
        draw_centered(renderer, back, BACK_GLYPH, &p.icon);
        let title_x = back.x + back.width + 12.0;
        draw_left(
            renderer,
            Rect {
                x: title_x,
                width: layout.width - PADDING - title_x,
                ..back
            },
            &page.title,
            &with_color(&p.label, p.text),
        );
    }

    for (i, (row, rect)) in page.rows.iter().zip(&layout.page_rows).enumerate() {
        let rect = *rect;
        match row {
            PageRow::Section(title) => {
                let section = TextStyle {
                    bold: true,
                    ..with_color(&p.small, p.dim)
                };
                let h = renderer.measure_text_height(title, &section);
                renderer.draw_text_ellipsized(
                    Rect {
                        x: rect.x + 4.0,
                        y: rect.y + rect.height - h - 4.0,
                        width: rect.width - 8.0,
                        height: h,
                    },
                    title,
                    &section,
                );
            }
            PageRow::Note(text) => {
                draw_left(
                    renderer,
                    Rect {
                        x: rect.x + 4.0,
                        width: rect.width - 8.0,
                        ..rect
                    },
                    text,
                    &with_color(&p.plain, p.dim),
                );
            }
            PageRow::Item {
                icon,
                label,
                detail,
                checked,
            } => {
                if is_hovered(PanelTarget::PageItem(i)) {
                    renderer.draw_rounded_rect(rect, PAGE_ITEM_RADIUS, style.surface);
                }
                let icon_color = if *checked { style.accent } else { p.text };
                draw_centered(
                    renderer,
                    Rect {
                        width: PAGE_ICON_WIDTH,
                        ..rect
                    },
                    icon,
                    &with_color(&p.icon, icon_color),
                );
                let right = draw_trailing(renderer, rect, detail.as_deref(), *checked, p);
                draw_left(
                    renderer,
                    Rect {
                        x: rect.x + PAGE_TEXT_X,
                        width: (right - rect.x - PAGE_TEXT_X - 8.0).max(0.0),
                        ..rect
                    },
                    label,
                    &with_color(&p.plain, p.text),
                );
            }
            PageRow::Slider {
                icon,
                label,
                value,
                muted,
                checked,
            } => {
                let label_rect = Rect {
                    height: PAGE_SLIDER_LABEL_HEIGHT,
                    ..rect
                };
                if checked.is_some() && is_hovered(PanelTarget::PageItem(i)) {
                    renderer.draw_rounded_rect(label_rect, 8.0, style.surface);
                }
                let right = draw_trailing(renderer, label_rect, None, *checked == Some(true), p);
                let label_color = if *checked == Some(true) {
                    style.accent
                } else {
                    p.text
                };
                draw_left(
                    renderer,
                    Rect {
                        x: rect.x + PAGE_TEXT_X,
                        width: (right - rect.x - PAGE_TEXT_X - 8.0).max(0.0),
                        ..label_rect
                    },
                    label,
                    &with_color(&p.small, label_color),
                );

                if let Some(button) = layout.rect_of(PanelTarget::PageSliderIcon(i)) {
                    if is_hovered(PanelTarget::PageSliderIcon(i)) {
                        renderer.draw_rounded_rect(button, button.height / 2.0, style.surface);
                    }
                    let color = if *muted { p.dim } else { p.text };
                    draw_centered(renderer, button, icon, &with_color(&p.icon, color));
                }
                if let Some(track) = layout.track_of(PanelTarget::PageSlider(i)) {
                    let fill = if *muted {
                        style.accent.with_alpha(style.accent.a * 0.4)
                    } else {
                        style.accent
                    };
                    draw_track(
                        renderer,
                        track,
                        *value,
                        fill,
                        style.surface,
                        is_hovered(PanelTarget::PageSlider(i)),
                    );
                }
            }
        }
    }
}

/// Draws the dim `detail` text and check mark at the right end of `rect`.
/// Returns the x where they start, so the label can stop before it.
fn draw_trailing<R: Renderer>(
    renderer: &mut R,
    rect: Rect,
    detail: Option<&str>,
    checked: bool,
    p: &Palette,
) -> f64 {
    let mut right = rect.x + rect.width - PAGE_INNER_PAD;
    if checked {
        let check = with_color(&p.icon, p.style.accent);
        let w = renderer.measure_text(CHECK_GLYPH, &check);
        right -= w;
        draw_left(
            renderer,
            Rect {
                x: right,
                width: w,
                ..rect
            },
            CHECK_GLYPH,
            &check,
        );
        right -= 8.0;
    }
    if let Some(detail) = detail.filter(|d| !d.is_empty()) {
        let style = with_color(&p.small, p.dim);
        let w = renderer.measure_text(detail, &style).min(rect.width / 2.0);
        right -= w;
        draw_left(
            renderer,
            Rect {
                x: right,
                width: w,
                ..rect
            },
            detail,
            &style,
        );
        right -= 8.0;
    }
    right
}

fn draw_track<R: Renderer>(
    renderer: &mut R,
    track: Rect,
    value: f64,
    fill: Color,
    empty: Color,
    hovered: bool,
) {
    let value = value.clamp(0.0, 1.0);
    let radius = SLIDER_TRACK_HEIGHT / 2.0;
    renderer.draw_rounded_rect(track, radius, empty);
    let (start, travel) = knob_travel(&track);
    let knob_x = start + travel * value;
    renderer.draw_rounded_rect(
        Rect {
            width: knob_x - track.x,
            ..track
        },
        radius,
        fill,
    );
    let knob_r = if hovered {
        SLIDER_KNOB_RADIUS + 1.0
    } else {
        SLIDER_KNOB_RADIUS
    };
    renderer.draw_rounded_rect(
        Rect {
            x: knob_x - knob_r,
            y: track.y + radius - knob_r,
            width: 2.0 * knob_r,
            height: 2.0 * knob_r,
        },
        knob_r,
        fill,
    );
}

/// A label with an optional smaller line under it, vertically centred in
/// `rect` and cut with an ellipsis when too wide.
fn draw_two_lines<R: Renderer>(
    renderer: &mut R,
    rect: Rect,
    label: &str,
    label_style: &TextStyle,
    subtitle: Option<&str>,
    sub_style: &TextStyle,
) {
    let label_h = renderer.measure_text_height(label, label_style);
    let Some(subtitle) = subtitle.filter(|s| !s.is_empty()) else {
        draw_left(renderer, rect, label, label_style);
        return;
    };
    let sub_h = renderer.measure_text_height(subtitle, sub_style);
    let top = rect.y + (rect.height - label_h - sub_h) / 2.0;
    renderer.draw_text_ellipsized(
        Rect {
            y: top,
            height: label_h,
            ..rect
        },
        label,
        label_style,
    );
    renderer.draw_text_ellipsized(
        Rect {
            y: top + label_h,
            height: sub_h,
            ..rect
        },
        subtitle,
        sub_style,
    );
}

/// One line of text at the left of `rect`, vertically centred, with an
/// ellipsis when it does not fit.
fn draw_left<R: Renderer>(renderer: &mut R, rect: Rect, text: &str, style: &TextStyle) {
    if text.is_empty() || rect.width <= 0.0 {
        return;
    }
    let h = renderer.measure_text_height(text, style);
    renderer.draw_text_ellipsized(
        Rect {
            y: rect.y + (rect.height - h) / 2.0,
            height: h,
            ..rect
        },
        text,
        style,
    );
}

fn inset(rect: Rect, by: f64) -> Rect {
    Rect {
        x: rect.x + by,
        y: rect.y + by,
        width: (rect.width - 2.0 * by).max(0.0),
        height: (rect.height - 2.0 * by).max(0.0),
    }
}

fn hover_color(color: Color, hovered: bool) -> Color {
    if hovered {
        color.mix(Color::WHITE, 0.10)
    } else {
        color
    }
}

fn with_color(style: &TextStyle, color: Color) -> TextStyle {
    TextStyle {
        color,
        ..style.clone()
    }
}

fn draw_centered<R: Renderer>(renderer: &mut R, rect: Rect, text: &str, style: &TextStyle) {
    if text.is_empty() {
        return;
    }
    let w = renderer.measure_text(text, style);
    let h = renderer.measure_text_height(text, style);
    renderer.draw_text(
        Point {
            x: rect.x + (rect.width - w) / 2.0,
            y: rect.y + (rect.height - h) / 2.0,
        },
        text,
        style,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> PanelStyle {
        PanelStyle {
            width: 400.0,
            corner_radius: 20.0,
            background: Color::BLACK,
            border: Color::BLACK,
            surface: Color::BLACK,
            accent: Color::WHITE,
            on_accent: Color::BLACK,
            danger: Color::WHITE,
        }
    }

    fn view() -> PanelView {
        let button = |align_right| PanelButton {
            icon: "x".into(),
            align_right,
            armed: false,
        };
        let tile = |has_menu| PanelTile {
            icon: "x".into(),
            label: "Tile".into(),
            subtitle: None,
            active: false,
            has_menu,
        };
        PanelView {
            buttons: vec![button(false), button(false), button(true), button(true)],
            sliders: vec![PanelSlider {
                icon: "v".into(),
                value: 0.5,
                has_menu: true,
            }],
            tiles: vec![tile(true), tile(false), tile(false)],
            media: None,
            page: None,
            style: style(),
        }
    }

    fn page_view() -> PanelView {
        PanelView {
            page: Some(PanelPage {
                title: "Sound".into(),
                rows: vec![
                    PageRow::Section("Output".into()),
                    PageRow::Slider {
                        icon: "s".into(),
                        label: "Speakers".into(),
                        value: 0.5,
                        muted: false,
                        checked: Some(true),
                    },
                    PageRow::Section("Apps".into()),
                    PageRow::Slider {
                        icon: "f".into(),
                        label: "Firefox".into(),
                        value: 1.0,
                        muted: true,
                        checked: None,
                    },
                    PageRow::Item {
                        icon: "g".into(),
                        label: "Settings".into(),
                        detail: None,
                        checked: false,
                    },
                    PageRow::Note("nothing".into()),
                ],
            }),
            ..view()
        }
    }

    #[test]
    fn right_buttons_are_flush_with_the_right_padding() {
        let layout = layout_panel(&view());
        let last = layout.rect_of(PanelTarget::Button(3)).unwrap();
        assert_eq!(last.x + last.width, 400.0 - PADDING);
        let first = layout.rect_of(PanelTarget::Button(0)).unwrap();
        assert_eq!(first.x, PADDING);
    }

    #[test]
    fn slider_hit_reports_position_along_track() {
        let layout = layout_panel(&view());
        let track = layout.track_of(PanelTarget::Slider(0)).unwrap();
        let y = track.y + 1.0;

        let (_, target, start) = layout.hit_test(track.x, y).unwrap();
        assert_eq!(target, PanelTarget::Slider(0));
        assert_eq!(start, 0.0);

        let (_, _, middle) = layout.hit_test(track.x + track.width / 2.0, y).unwrap();
        assert!((middle - 0.5).abs() < 1e-9);
    }

    #[test]
    fn knob_stays_inside_the_track_at_both_ends() {
        let layout = layout_panel(&view());
        let track = layout.track_of(PanelTarget::Slider(0)).unwrap();
        let (start, travel) = knob_travel(&track);
        assert_eq!(start - SLIDER_KNOB_RADIUS, track.x);
        assert_eq!(start + travel + SLIDER_KNOB_RADIUS, track.x + track.width);
        // Clicking either end of the track gives exactly 0 and 1.
        let y = track.y;
        assert_eq!(layout.hit_test(track.x, y).unwrap().2, 0.0);
        assert_eq!(
            layout.hit_test(track.x + track.width - 0.5, y).unwrap().2,
            1.0
        );
    }

    #[test]
    fn drag_clamps_outside_the_track() {
        let layout = layout_panel(&view());
        let track = layout.track_of(PanelTarget::Slider(0)).unwrap();
        let (index, _, _) = layout.hit_test(track.x + 5.0, track.y).unwrap();

        assert_eq!(layout.drag(index, -100.0).unwrap().1, 0.0);
        assert_eq!(layout.drag(index, 10_000.0).unwrap().1, 1.0);
    }

    #[test]
    fn tile_with_menu_splits_into_two_targets() {
        let layout = layout_panel(&view());
        let main = layout.rect_of(PanelTarget::Tile(0)).unwrap();
        let arrow = layout.rect_of(PanelTarget::TileMenu(0)).unwrap();
        assert_eq!(main.x + main.width, arrow.x);
        assert_eq!(arrow.width, TILE_ARROW_WIDTH);

        let (_, target, _) = layout.hit_test(arrow.x + 2.0, arrow.y + 2.0).unwrap();
        assert_eq!(target, PanelTarget::TileMenu(0));
        assert!(layout.rect_of(PanelTarget::TileMenu(1)).is_none());
    }

    #[test]
    fn odd_tile_count_adds_a_row() {
        let layout = layout_panel(&view());
        let third = layout.rect_of(PanelTarget::Tile(2)).unwrap();
        let first = layout.rect_of(PanelTarget::Tile(0)).unwrap();
        assert_eq!(third.y, first.y + TILE_HEIGHT + TILE_GAP);
        assert_eq!(layout.height, (third.y + TILE_HEIGHT + PADDING).ceil());
    }

    #[test]
    fn gaps_between_targets_hit_nothing() {
        let layout = layout_panel(&view());
        assert!(layout.hit_test(1.0, 1.0).is_none());
    }

    #[test]
    fn page_replaces_the_main_layout() {
        let layout = layout_panel(&page_view());
        assert!(layout.rect_of(PanelTarget::Button(0)).is_none());
        assert!(layout.rect_of(PanelTarget::Tile(0)).is_none());
        assert!(layout.rect_of(PanelTarget::Back).is_some());
        assert_eq!(layout.width, 400.0);
    }

    #[test]
    fn page_slider_has_label_mute_and_track_targets() {
        let layout = layout_panel(&page_view());
        // Selectable slider: label line, mute button and track.
        let label = layout.rect_of(PanelTarget::PageItem(1)).unwrap();
        let icon = layout.rect_of(PanelTarget::PageSliderIcon(1)).unwrap();
        let track = layout.track_of(PanelTarget::PageSlider(1)).unwrap();
        assert!(label.y + label.height <= icon.y);
        assert!(icon.x + icon.width <= track.x);

        let (_, target, fraction) = layout
            .hit_test(track.x + track.width - 0.5, track.y)
            .unwrap();
        assert_eq!(target, PanelTarget::PageSlider(1));
        assert_eq!(fraction, 1.0);

        // A slider without `checked` has no clickable label.
        assert!(layout.rect_of(PanelTarget::PageItem(3)).is_none());
        assert!(layout.rect_of(PanelTarget::PageSlider(3)).is_some());
    }

    #[test]
    fn page_items_are_clickable_but_sections_and_notes_are_not() {
        let layout = layout_panel(&page_view());
        let item = layout.rect_of(PanelTarget::PageItem(4)).unwrap();
        let (_, target, _) = layout.hit_test(item.x + 10.0, item.y + 10.0).unwrap();
        assert_eq!(target, PanelTarget::PageItem(4));
        assert!(layout.rect_of(PanelTarget::PageItem(0)).is_none());
        assert!(layout.rect_of(PanelTarget::PageItem(5)).is_none());

        let note = layout.page_rows[5];
        assert_eq!(layout.height, (note.y + note.height + PADDING).ceil());
    }

    fn media_view(progress: Option<f64>) -> PanelView {
        PanelView {
            media: Some(PanelMedia {
                icon: "m".into(),
                art: None,
                title: "Song".into(),
                artist: Some("Artist".into()),
                playing: true,
                progress,
            }),
            ..view()
        }
    }

    #[test]
    fn media_card_sits_between_buttons_and_sliders() {
        let layout = layout_panel(&media_view(Some(0.5)));
        let card = layout.media_card.unwrap();
        let button = layout.rect_of(PanelTarget::Button(0)).unwrap();
        let slider = layout.rect_of(PanelTarget::Slider(0)).unwrap();
        assert_eq!(card.y, button.y + BUTTON_SIZE + SECTION_GAP);
        assert_eq!(slider.y, card.y + MEDIA_HEIGHT + SECTION_GAP);

        // Controls are flush right inside the card, in order.
        let prev = layout.rect_of(PanelTarget::MediaPrevious).unwrap();
        let play = layout.rect_of(PanelTarget::MediaPlayPause).unwrap();
        let next = layout.rect_of(PanelTarget::MediaNext).unwrap();
        assert!(prev.x < play.x && play.x < next.x);
        assert_eq!(next.x + next.width, card.x + card.width - MEDIA_PAD);

        let without = layout_panel(&view());
        assert_eq!(layout.height, without.height + MEDIA_HEIGHT + SECTION_GAP);
    }

    #[test]
    fn seek_bar_maps_edge_to_edge_and_drags() {
        let layout = layout_panel(&media_view(Some(0.2)));
        let track = layout.track_of(PanelTarget::MediaSeek).unwrap();
        let (index, target, start) = layout.hit_test(track.x, track.y + 1.0).unwrap();
        assert_eq!(target, PanelTarget::MediaSeek);
        assert!(target.is_slider());
        assert_eq!(start, 0.0);
        let (_, _, middle) = layout
            .hit_test(track.x + track.width / 2.0, track.y)
            .unwrap();
        assert!((middle - 0.5).abs() < 1e-9);
        assert_eq!(layout.drag(index, 1e6).unwrap().1, 1.0);
    }

    #[test]
    fn no_seek_bar_without_progress() {
        let layout = layout_panel(&media_view(None));
        assert!(layout.rect_of(PanelTarget::MediaSeek).is_none());
        assert!(layout.rect_of(PanelTarget::MediaPlayPause).is_some());
    }
}
