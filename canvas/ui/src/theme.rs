//! Global visual tokens — the single source of truth for AURI's style.

// Palette (RGB integers for `rgb(...)`).
pub const COLOR_BASE: u32 = 0x101112;
pub const COLOR_PANEL: u32 = 0x18191B;
pub const COLOR_ELEVATED: u32 = 0x222326;
pub const COLOR_BORDER: u32 = 0x35373A;
pub const COLOR_FOREGROUND: u32 = 0xE8E8E6;
pub const COLOR_MUTED: u32 = 0x9C9D9A;
pub const COLOR_ACCENT: u32 = 0xC9A227;
pub const COLOR_DANGER: u32 = 0xE06C63;
pub const COLOR_SELECTION: u32 = 0x7E96AE;
pub const COLOR_GRID: u32 = 0x2D2F32;
pub const COLOR_SUBTLE: u32 = 0x424446;
pub const COLOR_NOTE_BG: u32 = 0xD8CBA6;
pub const COLOR_NOTE_FG: u32 = 0x2A2721;
pub const COLOR_NOTE_PLACEHOLDER: u32 = 0x6B6553;
pub const COLOR_SUCCESS: u32 = 0x6F9E7A;

// Workspace row (translucent gray, not a solid accent).
pub const COLOR_ROW_BORDER_SOFT: u32 = 0x8A8D9133;
pub const COLOR_ROW_BORDER_ACTIVE: u32 = 0x9A9DA266;
pub const WORKSPACE_ROW_PAD_Y: f32 = 5.;
pub const DRAWER_HANDLE_WIDTH: f32 = 4.;
pub const MIN_DRAWER_WIDTH: f32 = 160.;
pub const MAX_DRAWER_WIDTH: f32 = 420.;
pub const ROW_ICON_SIZE: f32 = 12.;

// Radii and icons.
pub const RADIUS_CHIP: f32 = 4.;
pub const RADIUS_CONTROL: f32 = 8.;
pub const RADIUS_DIALOG: f32 = 14.;
pub const ICON_SIZE: f32 = 16.;
pub const COLOR_SWATCH_SIZE: f32 = 20.;
pub const COUNT_BADGE_OFFSET: f32 = -3.;
pub const COUNT_BADGE_RESERVE: f32 = 8.;

// Spacing scale.
pub const SPACE_TIGHT: f32 = 4.;
pub const SPACE_NORMAL: f32 = 8.;
pub const SPACE_WIDE: f32 = 12.;
pub const SPACE_PANEL: f32 = 16.;

// Component dimensions.
pub const DRAWER_WIDTH: f32 = 224.;
pub const FONT_TERMINAL: f32 = 13.;
pub const LINE_HEIGHT_TERMINAL: f32 = 1.2;
pub const TITLE_HEIGHT: f32 = 32.;
pub const PATH_HEIGHT: f32 = 22.;
pub const HANDLE: f32 = 12.;
pub const MIN_TERMINAL_WIDTH: f32 = 320.;
pub const MIN_TERMINAL_HEIGHT: f32 = 200.;
pub const CARD_WIDTH: f32 = 640.;
pub const CARD_HEIGHT: f32 = 400.;
pub const GRID_STEP: f32 = 24.;

// Context menu.
pub const CONTEXT_MENU_WIDTH: f32 = 180.;
pub const CONTEXT_MENU_ITEM_HEIGHT: f32 = 32.;
pub const SUBMENU_GAP: f32 = 4.;
pub const TEXT_CARD_WIDTH: f32 = 320.;
pub const TEXT_CARD_HEIGHT: f32 = 220.;
pub const TEXT_CARD_PADDING: f32 = 12.;

use canvas_types::WorkspaceColor;

/// Map a workspace's persisted color choice to a concrete swatch.
pub fn workspace_color(color: WorkspaceColor) -> u32 {
    match color {
        WorkspaceColor::Brass => COLOR_ACCENT,
        WorkspaceColor::Steel => COLOR_SELECTION,
        WorkspaceColor::Red => COLOR_DANGER,
        WorkspaceColor::Green => COLOR_SUCCESS,
    }
}
