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

// Radii and icons.
pub const RADIUS_CHIP: f32 = 4.;
pub const RADIUS_CONTROL: f32 = 8.;
pub const RADIUS_DIALOG: f32 = 14.;
pub const ICON_SIZE: f32 = 16.;

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
pub const CONTEXT_MENU_WIDTH_WIDE: f32 = 216.;
pub const DRAWER_MENU_TOP: f32 = 56.;
