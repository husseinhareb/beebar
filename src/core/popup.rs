use crate::renderer::color::Color;
use crate::renderer::primitives::{Point, Rect, Renderer, TextStyle};

pub const POPUP_GAP: f64 = 6.0;
const POPUP_EDGE_MARGIN: f64 = 4.0;
const POPUP_BORDER: f64 = 1.0;
const POPUP_HORIZONTAL_PADDING: f64 = 12.0;
const POPUP_VERTICAL_PADDING: f64 = 8.0;
const POPUP_ROW_VERTICAL_PADDING: f64 = 6.0;
const POPUP_MIN_WIDTH: f64 = 96.0;
/// Total height of a separator row (the visible 1-px line is centred in it).
const POPUP_SEPARATOR_HEIGHT: f64 = 9.0;
/// Width reserved on the right edge for the submenu chevron.
const POPUP_CHEVRON_WIDTH: f64 = 16.0;
/// Width reserved on the left edge for checkbox glyphs.
const POPUP_CHECKBOX_WIDTH: f64 = 18.0;

const POPUP_BACKGROUND: Color = Color::rgb(0.08, 0.09, 0.12);
const POPUP_BORDER_COLOR: Color = Color::rgb(0.24, 0.27, 0.32);
const POPUP_DISABLED_TEXT: Color = Color::rgb(0.45, 0.47, 0.51);
const POPUP_HOVER_BG: Color = Color::rgb(0.16, 0.18, 0.24);
const POPUP_SEPARATOR_COLOR: Color = Color::rgb(0.24, 0.27, 0.32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupItemKind {
    Action,
    Separator,
    Checkbox(bool),
    Submenu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupMenuItem {
    pub label: String,
    pub enabled: bool,
    pub kind: PopupItemKind,
}

impl PopupMenuItem {
    pub fn action(label: impl Into<String>, enabled: bool) -> Self {
        Self {
            label: label.into(),
            enabled,
            kind: PopupItemKind::Action,
        }
    }

    pub fn is_selectable(&self) -> bool {
        !matches!(self.kind, PopupItemKind::Separator)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PopupMenu {
    /// Horizontal anchor in bar-local coordinates.
    pub anchor_x: f64,
    pub items: Vec<PopupMenuItem>,
}

#[derive(Debug, Clone)]
pub struct PopupLayout {
    pub x: f64,
    pub width: f64,
    pub height: f64,
    item_offsets: Vec<f64>,
    item_sizes: Vec<f64>,
    text_y_offset: f64,
    label_x: f64,
}

impl PopupLayout {
    pub fn pixel_width(&self) -> u32 {
        self.width.ceil().max(1.0) as u32
    }

    pub fn pixel_height(&self) -> u32 {
        self.height.ceil().max(1.0) as u32
    }

    pub fn hit_test(&self, items: &[PopupMenuItem], x: f64, y: f64) -> Option<usize> {
        if items.is_empty() || x < 0.0 || y < 0.0 || x >= self.width || y >= self.height {
            return None;
        }
        for (idx, item) in items.iter().enumerate() {
            if !item.is_selectable() {
                continue;
            }
            let top = self.item_offsets[idx];
            let bot = top + self.item_sizes[idx];
            if y >= top && y < bot {
                return Some(idx);
            }
        }
        None
    }

    fn row_y(&self, index: usize) -> f64 {
        self.item_offsets[index]
    }

    fn row_height(&self, index: usize) -> f64 {
        self.item_sizes[index]
    }
}

pub fn layout_popup<R: Renderer>(
    popup: &PopupMenu,
    bar_width: f64,
    style: &TextStyle,
    renderer: &R,
) -> Option<PopupLayout> {
    if popup.items.is_empty() {
        return None;
    }

    let text_height = popup
        .items
        .iter()
        .filter(|item| !matches!(item.kind, PopupItemKind::Separator))
        .map(|item| renderer.measure_text_height(&item.label, style))
        .fold(renderer.measure_text_height("Ag", style), f64::max)
        .max(1.0);

    let needs_check_col = popup
        .items
        .iter()
        .any(|item| matches!(item.kind, PopupItemKind::Checkbox(_)));
    let needs_chevron_col = popup
        .items
        .iter()
        .any(|item| matches!(item.kind, PopupItemKind::Submenu));

    let left_pad = POPUP_HORIZONTAL_PADDING
        + if needs_check_col {
            POPUP_CHECKBOX_WIDTH
        } else {
            0.0
        };
    let right_pad = POPUP_HORIZONTAL_PADDING
        + if needs_chevron_col {
            POPUP_CHEVRON_WIDTH
        } else {
            0.0
        };

    let text_width = popup
        .items
        .iter()
        .filter(|item| !matches!(item.kind, PopupItemKind::Separator))
        .map(|item| renderer.measure_text(&item.label, style))
        .fold(0.0, f64::max);

    let action_height = (text_height + POPUP_ROW_VERTICAL_PADDING * 2.0).ceil();
    let width = (text_width + left_pad + right_pad + POPUP_BORDER * 2.0)
        .max(POPUP_MIN_WIDTH)
        .ceil();

    let mut item_offsets = Vec::with_capacity(popup.items.len());
    let mut item_sizes = Vec::with_capacity(popup.items.len());
    let mut cursor = POPUP_BORDER + POPUP_VERTICAL_PADDING;
    for item in &popup.items {
        let h = match item.kind {
            PopupItemKind::Separator => POPUP_SEPARATOR_HEIGHT,
            _ => action_height,
        };
        item_offsets.push(cursor);
        item_sizes.push(h);
        cursor += h;
    }
    let height = (cursor + POPUP_VERTICAL_PADDING + POPUP_BORDER).ceil();

    let max_x = (bar_width - width - POPUP_EDGE_MARGIN).max(POPUP_EDGE_MARGIN);
    let x = if max_x <= POPUP_EDGE_MARGIN {
        ((bar_width - width) / 2.0).max(0.0)
    } else {
        (popup.anchor_x - width / 2.0).clamp(POPUP_EDGE_MARGIN, max_x)
    };

    Some(PopupLayout {
        x,
        width,
        height,
        item_offsets,
        item_sizes,
        text_y_offset: (action_height - text_height) / 2.0,
        label_x: POPUP_BORDER + left_pad,
    })
}

pub fn draw_popup<R: Renderer>(
    renderer: &mut R,
    popup: &PopupMenu,
    layout: &PopupLayout,
    style: &TextStyle,
    hovered: Option<usize>,
) {
    renderer.draw_rect(
        Rect {
            x: 0.0,
            y: 0.0,
            width: layout.width,
            height: layout.height,
        },
        POPUP_BACKGROUND,
    );

    renderer.draw_rect(
        Rect {
            x: 0.0,
            y: 0.0,
            width: layout.width,
            height: POPUP_BORDER,
        },
        POPUP_BORDER_COLOR,
    );
    renderer.draw_rect(
        Rect {
            x: 0.0,
            y: layout.height - POPUP_BORDER,
            width: layout.width,
            height: POPUP_BORDER,
        },
        POPUP_BORDER_COLOR,
    );
    renderer.draw_rect(
        Rect {
            x: 0.0,
            y: 0.0,
            width: POPUP_BORDER,
            height: layout.height,
        },
        POPUP_BORDER_COLOR,
    );
    renderer.draw_rect(
        Rect {
            x: layout.width - POPUP_BORDER,
            y: 0.0,
            width: POPUP_BORDER,
            height: layout.height,
        },
        POPUP_BORDER_COLOR,
    );

    for (idx, item) in popup.items.iter().enumerate() {
        let row_y = layout.row_y(idx);
        let row_h = layout.row_height(idx);

        if hovered == Some(idx) && item.is_selectable() {
            renderer.draw_rect(
                Rect {
                    x: POPUP_BORDER,
                    y: row_y,
                    width: layout.width - POPUP_BORDER * 2.0,
                    height: row_h,
                },
                POPUP_HOVER_BG,
            );
        }

        match item.kind {
            PopupItemKind::Separator => {
                renderer.draw_rect(
                    Rect {
                        x: POPUP_BORDER + POPUP_HORIZONTAL_PADDING,
                        y: row_y + (row_h - 1.0) / 2.0,
                        width: layout.width
                            - POPUP_BORDER * 2.0
                            - POPUP_HORIZONTAL_PADDING * 2.0,
                        height: 1.0,
                    },
                    POPUP_SEPARATOR_COLOR,
                );
            }
            _ => {
                let mut item_style = style.clone();
                if !item.enabled {
                    item_style.color = POPUP_DISABLED_TEXT;
                }

                if let PopupItemKind::Checkbox(checked) = item.kind {
                    let glyph = if checked { "\u{2611}" } else { "\u{2610}" };
                    renderer.draw_text(
                        Point {
                            x: POPUP_BORDER + POPUP_HORIZONTAL_PADDING,
                            y: row_y + layout.text_y_offset,
                        },
                        glyph,
                        &item_style,
                    );
                }

                renderer.draw_text(
                    Point {
                        x: layout.label_x,
                        y: row_y + layout.text_y_offset,
                    },
                    &item.label,
                    &item_style,
                );

                if matches!(item.kind, PopupItemKind::Submenu) {
                    let glyph = "\u{203A}";
                    let glyph_w = renderer.measure_text(glyph, &item_style);
                    renderer.draw_text(
                        Point {
                            x: layout.width
                                - POPUP_BORDER
                                - POPUP_HORIZONTAL_PADDING
                                - glyph_w,
                            y: row_y + layout.text_y_offset,
                        },
                        glyph,
                        &item_style,
                    );
                }
            }
        }
    }
}
