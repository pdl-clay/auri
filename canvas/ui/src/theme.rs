//! Global visual tokens — the single source of truth for AURI's style.

// Palette (RGB integers for `rgb(...)`).
pub const COLOR_BASE: u32 = 0x101112;
pub const COLOR_PANEL: u32 = 0x18191B;
pub const COLOR_ELEVATED: u32 = 0x222326;
pub const COLOR_BORDER: u32 = 0x35373A;
pub const COLOR_FOREGROUND: u32 = 0xE8E8E6;
pub const COLOR_MUTED: u32 = 0x9C9D9A;
pub const COLOR_UNLOADED_ICON: u32 = 0x4E5052;
pub const COLOR_ACCENT: u32 = 0xC9A227;
pub const COLOR_DANGER: u32 = 0xE06C63;
pub const COLOR_SELECTION: u32 = 0x7E96AE;
pub const COLOR_GRID: u32 = 0x3B3E42;
pub const COLOR_GRID_ALPHA: f32 = 0.2;
pub const COLOR_SUBTLE: u32 = 0x424446;
pub const COLOR_NOTE_BG: u32 = 0xD8CBA6;
pub const COLOR_NOTE_FG: u32 = 0x2A2721;
pub const COLOR_NOTE_PLACEHOLDER: u32 = 0x6B6553;
pub const COLOR_SUCCESS: u32 = 0x6F9E7A;

// Agent card accents — every agent colour lives here, never in a component.
pub const COLOR_AGENT_CLAUDE: u32 = 0xD97757;
pub const COLOR_AGENT_CODEX: u32 = 0x19A58C;
pub const COLOR_AGENT_PI: u32 = 0xA78BFA;
pub const COLOR_AGENT_OMP: u32 = 0xE2B35C;
pub const COLOR_AGENT_OPEN_CODE: u32 = 0x4FB3E8;

// Workspace rows read as text until hovered; only the selected row keeps a
// border. Hover is one small step lighter than the drawer, not a filled card.
pub const COLOR_ROW_HOVER: u32 = 0x1F2022;
pub const COLOR_ROW_BORDER_ACTIVE: u32 = 0x9A9DA266;
pub const WORKSPACE_ROW_PAD_Y: f32 = 2.;
pub const WORKSPACE_ROW_GAP: f32 = 0.;
pub const DRAWER_HANDLE_WIDTH: f32 = 4.;
pub const MIN_DRAWER_WIDTH: f32 = 160.;
pub const MAX_DRAWER_WIDTH: f32 = 420.;
pub const ROW_ICON_SIZE: f32 = 12.;

// Radii and icons.
pub const RADIUS_CHIP: f32 = 4.;
pub const RADIUS_CONTROL: f32 = 8.;
pub const RADIUS_DIALOG: f32 = 14.;
/// All interface text fields are pills, matching the chat composer.
pub const RADIUS_INPUT: f32 = RADIUS_PILL;
/// Fully rounded pill: the standard for text fields and the picker action.
pub const RADIUS_PILL: f32 = 999.;
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
pub const FONT_CARD_TITLE: f32 = 12.;
pub const FONT_CARD_PATH: f32 = 11.;
pub const FONT_NOTE_TITLE: f32 = 11.;
pub const FONT_NOTE_BODY: f32 = 13.;
pub const LINE_HEIGHT_TERMINAL: f32 = 1.2;
/// A header is a strip, not a row: one line of 11–12px text with 4px of air
/// above and below the close control. Growing this token is what made the
/// window frame look heavy; the contract is pinned by `regressions.rs`.
pub const TITLE_HEIGHT: f32 = 32.;
pub const PATH_HEIGHT: f32 = 22.;
pub const HANDLE: f32 = 12.;
pub const CLOSE_BUTTON_SIZE: f32 = 24.;
pub const CLOSE_ICON_SIZE: f32 = 12.;
pub const MIN_TERMINAL_WIDTH: f32 = 240.;
pub const MIN_TERMINAL_HEIGHT: f32 = 140.;
pub const CARD_WIDTH: f32 = 960.;
pub const CARD_HEIGHT: f32 = 600.;
pub const GRID_STEP: f32 = 36.;
pub const CARD_GAP: f32 = 1.;

// Zoom island and minimap. All visual dimensions live here, never inline.
pub const RADIUS_ISLAND: f32 = 20.;
pub const ZOOM_CONTROL_SIZE: f32 = 24.;
pub const ZOOM_PERCENT_WIDTH: f32 = 38.;
pub const MINIMAP_WIDTH: f32 = 232.;
pub const MINIMAP_HEIGHT: f32 = 152.;
pub const MINIMAP_PADDING: f32 = 10.;
pub const MINIMAP_MARGIN_RIGHT: f32 = 12.;
pub const MINIMAP_MARGIN_BOTTOM: f32 = 52.;
pub const MINIMAP_WORLD_PADDING: f32 = 24.;
pub const MINIMAP_ITEM_MIN_SIZE: f32 = 2.;
pub const MINIMAP_ITEM_RADIUS: f32 = 2.;

// Connection gesture and persisted edges.
pub const CONNECTION_LINE_COLOR: u32 = 0x9AB4CE;
pub const CONNECTION_SOURCE_COLOR: u32 = 0xC9A227;
pub const CONNECTION_LINE_WIDTH: f32 = 2.0;
pub const CONNECTION_DASH_LENGTH: f32 = 8.0;
pub const CONNECTION_DASH_GAP: f32 = 6.0;

// A short pulse of light runs along a rope when an agent sends something
// across it. Same blue family as the rope, one step brighter for the core.
pub const CONNECTION_PULSE_COLOR: u32 = 0xDCEBFF;
pub const CONNECTION_PULSE_HALO: u32 = 0x9AB4CE;
pub const CONNECTION_PULSE_HALO_ALPHA: f32 = 0.33;
pub const CONNECTION_PULSE_SIZE: f32 = 9.;
pub const CONNECTION_PULSE_HALO_SIZE: f32 = 22.;
pub const CONNECTION_PULSE_SECOND_SIZE: f32 = 5.;
pub const CONNECTION_PULSE_SECOND_OFFSET: f32 = 0.045;
pub const CONNECTION_PULSE_SECOND_ALPHA: f32 = 0.85;
pub const CONNECTION_PULSE_TRAIL_PUFFS: usize = 6;
pub const CONNECTION_PULSE_TRAIL_STEP: f32 = 0.028;
pub const CONNECTION_PULSE_TRAIL_SIZE: f32 = 18.;
pub const CONNECTION_PULSE_TRAIL_ALPHA: f32 = 0.24;
pub const CONNECTION_PULSE_ROPE_GLOW_WIDTH: f32 = 5.;
pub const CONNECTION_PULSE_ROPE_GLOW_ALPHA: f32 = 0.22;
pub const CONNECTION_PULSE_DURATION_MS: u64 = 2000;

// Connection inspector opened by pressing a rope.
pub const CONNECTION_HIT_RADIUS: f32 = 8.;
pub const CONNECTION_POPOVER_WIDTH: f32 = 240.;
pub const CONNECTION_POPOVER_HEIGHT: f32 = 168.;
pub const CONNECTION_POPOVER_FONT: f32 = 12.;
pub const CONNECTION_POPOVER_CLOSE_SIZE: f32 = 20.;
pub const CONNECTION_POPOVER_ARROW_INDENT: f32 = ICON_SIZE + 2.;
pub const CONNECTION_POPOVER_HINT_FONT: f32 = 11.;
pub const CONNECTION_POPOVER_ACTION_HEIGHT: f32 = 28.;

// Cross-workspace proxy: a compact, inert representation of a remote endpoint.
// Every dimension and colour it uses lives here, never inline in the renderer.
pub const PROXY_CARD_WIDTH: f32 = 240.;
pub const PROXY_CARD_HEIGHT: f32 = 56.;
pub const PROXY_CARD_RADIUS: f32 = RADIUS_CONTROL;
pub const PROXY_CARD_PADDING: f32 = SPACE_WIDE;
pub const PROXY_CARD_GAP: f32 = SPACE_NORMAL;
pub const PROXY_CARD_ICON_SIZE: f32 = ICON_SIZE;
pub const FONT_PROXY_NAME: f32 = FONT_CARD_TITLE;
pub const FONT_PROXY_SUBTITLE: f32 = 10.;
pub const COLOR_PROXY_SUBTITLE: u32 = 0xA8B4C4;
pub const COLOR_PROXY_BORDER: u32 = 0x5B7896;

/// Empty-canvas notice shown when no workspace has been loaded.
pub const WORKSPACE_NOTICE_WIDTH: f32 = 400.;
/// Compact card that reports a verified release without blocking the canvas.
pub const UPDATE_CARD_WIDTH: f32 = 320.;

// Agent chat view. The terminal stays behind it; these values only style the
// conversation layer.
pub const COLOR_CHAT_USER_BG: u32 = 0x0F6B4E;
pub const COLOR_CHAT_AGENT_BG: u32 = 0x1E4F7A;
pub const CHAT_BUBBLE_RADIUS: f32 = 12.;
pub const CHAT_BUBBLE_PADDING: f32 = 12.;
pub const CHAT_BUBBLE_TAIL_RADIUS: f32 = 4.;
pub const CHAT_TAIL_WIDTH: f32 = 10.;
pub const CHAT_TAIL_HEIGHT: f32 = 12.;
pub const CHAT_TAIL_OVERLAP: f32 = 1.;
pub const CHAT_BUBBLE_GAP: f32 = 10.;
pub const CHAT_BUBBLE_WIDTH: f32 = 0.82;
pub const CHAT_MESSAGE_IMAGE_W: f32 = 96.;
pub const CHAT_MESSAGE_IMAGE_H: f32 = 64.;
pub const CHAT_TEXT_SIZE: f32 = FONT_NOTE_BODY;
pub const CHAT_STATUS_FONT: f32 = FONT_CARD_PATH;
pub const CHAT_COPY_HEIGHT: f32 = 22.;
pub const CHAT_SKILLS_MAX_HEIGHT: f32 = 180.;
/// Keyboard navigation reaches exactly the rows the panel shows; the list is
/// capped instead of becoming an unbounded overlay.
pub const CHAT_SKILLS_VISIBLE: usize = 8;
pub const CHAT_COPY_RESERVE: f32 = 36.;
pub const CHAT_CODE_FONT: f32 = 11.;
pub const CHAT_CODE_PADDING: f32 = 12.;
pub const CHAT_SKILL_GAP: f32 = 2.;
pub const CHAT_ACTION_SIZE: f32 = 37.;
pub const CHAT_ATTACH_ICON: f32 = 19.;
pub const CHAT_ATTACH_PREVIEW: f32 = 64.;
pub const CHAT_ATTACH_PREVIEW_ICON: f32 = 24.;
pub const CHAT_ATTACH_LABEL_FONT: f32 = 9.;
pub const CHAT_ATTACH_REMOVE: f32 = 16.;
pub const CHAT_ATTACH_REMOVE_ICON: f32 = 10.;
/// Images larger than this fall back to the file tile instead of decoding.
pub const CHAT_ATTACH_IMAGE_MAX_BYTES: u64 = 32 * 1024 * 1024;
pub const CHAT_STOP_ICON: f32 = 20.;
pub const COLOR_CHAT_STOP_BG: u32 = 0xD64541;
pub const COLOR_CHAT_STOP_FG: u32 = 0xFFFFFF;
pub const COLOR_CHAT_ASK_OPTION: u32 = 0x2A5F8F;
pub const COLOR_CHAT_ASK_OPTION_HOVER: u32 = 0x26537D;
pub const CHAT_WAVE_DOT: f32 = 4.;
pub const CHAT_WAVE_GAP: f32 = 3.;
pub const CHAT_WAVE_HEIGHT: f32 = 14.;
pub const CHAT_WAVE_AMPLITUDE: f32 = 3.;
pub const CHAT_WAVE_DURATION_MS: u64 = 900;
pub const CHAT_STATUS_GAP: f32 = 6.;
pub const CHAT_STATUS_LABEL_FONT: f32 = 12.;
pub const CHAT_STATUS_PADDING: f32 = 8.;
pub const CHAT_MODEL_BAR_HEIGHT: f32 = 34.;
pub const CHAT_MODEL_CHIP_HEIGHT: f32 = 24.;
pub const CHAT_MODEL_FONT: f32 = 11.;
pub const CHAT_MODEL_NAME_FONT: f32 = 12.;
pub const CHAT_MODEL_PROVIDER_FONT: f32 = 10.;
pub const CHAT_MODEL_MENU_WIDTH: f32 = 460.;
pub const CHAT_MODEL_MENU_HEIGHT: f32 = 380.;
pub const CHAT_MODEL_PROVIDER_WIDTH: f32 = 120.;
pub const CHAT_MODEL_PROVIDER_ROW_FONT: f32 = 11.;
pub const CHAT_MODEL_PROVIDER_COUNT_FONT: f32 = 10.;
pub const CHAT_MODEL_DIVIDER_WIDTH: f32 = 1.;
pub const CHAT_MODEL_ROW_GAP: f32 = 2.;
/// Small status dot on the provider that owns the CLI's current model.
pub const CHAT_MODEL_IN_USE_DOT: f32 = 6.;
/// Context ring is 4px larger than the other 37px composer actions.
pub const CHAT_CONTEXT_SIZE: f32 = CHAT_ACTION_SIZE + 4.;
/// Twenty-eight radial bars, as in the approved gauge mock.
pub const CHAT_CONTEXT_BARS: usize = 28;
pub const CHAT_CONTEXT_BAR_WIDTH: f32 = 2.;
pub const CHAT_CONTEXT_BAR_LENGTH: f32 = 6.5;
pub const CHAT_CONTEXT_FONT: f32 = 10.;
/// Gradient stops: green until half, gold at 80%, red beyond.
pub const CHAT_CONTEXT_CALM: f32 = 0.5;
pub const CHAT_CONTEXT_WARNING: f32 = 0.8;

// Context menu.
pub const CONTEXT_MENU_WIDTH: f32 = 180.;
pub const CONTEXT_MENU_ITEM_HEIGHT: f32 = 32.;
pub const SUBMENU_GAP: f32 = 4.;
pub const TEXT_CARD_WIDTH: f32 = 320.;
pub const TEXT_CARD_HEIGHT: f32 = 220.;
pub const MIN_NOTE_WIDTH: f32 = 120.;
pub const MIN_NOTE_HEIGHT: f32 = 90.;
pub const TEXT_CARD_PADDING: f32 = 12.;
pub const AGENT_DIALOG_WIDTH: f32 = 340.;
pub const AGENT_DIALOG_HEIGHT: f32 = 320.;
/// The edit form has the extra admin switch row, so it needs a taller token.
pub const AGENT_EDIT_DIALOG_HEIGHT: f32 = 508.;
pub const AGENT_TILE_SIZE: f32 = 32.;
/// Small admin/member badge after the agent name in the card header.
pub const AGENT_ROLE_ICON: f32 = 12.;

use canvas_types::{AgentIcon, WorkspaceColor};

/// Map a workspace's persisted color choice to a concrete swatch.
pub fn workspace_color(color: WorkspaceColor) -> u32 {
    match color {
        WorkspaceColor::Brass => COLOR_ACCENT,
        WorkspaceColor::Steel => COLOR_SELECTION,
        WorkspaceColor::Red => COLOR_DANGER,
        WorkspaceColor::Green => COLOR_SUCCESS,
    }
}

/// Colour for one position along the gauge: green → gold → red.
pub fn context_color(fraction: f32) -> u32 {
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= CHAT_CONTEXT_CALM {
        mix_color(
            COLOR_SUCCESS,
            COLOR_ACCENT,
            fraction / CHAT_CONTEXT_CALM,
        )
    } else if fraction <= CHAT_CONTEXT_WARNING {
        mix_color(
            COLOR_ACCENT,
            COLOR_DANGER,
            (fraction - CHAT_CONTEXT_CALM) / (CHAT_CONTEXT_WARNING - CHAT_CONTEXT_CALM),
        )
    } else {
        COLOR_DANGER
    }
}

/// Interpolate two theme colours while the send button turns into pause.
pub fn mix_color(from: u32, to: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let a = ((from >> shift) & 0xff) as f32;
        let b = ((to >> shift) & 0xff) as f32;
        ((a + (b - a) * t).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// Accent used for an agent's icon and menu tile.
pub fn agent_color(icon: AgentIcon) -> u32 {
    match icon {
        AgentIcon::Claude => COLOR_AGENT_CLAUDE,
        AgentIcon::Codex => COLOR_AGENT_CODEX,
        AgentIcon::Pi => COLOR_AGENT_PI,
        AgentIcon::Omp => COLOR_AGENT_OMP,
        AgentIcon::OpenCode => COLOR_AGENT_OPEN_CODE,
    }
}

/// Embedded SVG matching the agent's mark.
pub fn agent_icon_path(icon: AgentIcon) -> &'static str {
    match icon {
        AgentIcon::Claude => "agent-claude.svg",
        AgentIcon::Codex => "agent-codex.svg",
        AgentIcon::Pi => "agent-pi.svg",
        AgentIcon::Omp => "agent-omp.svg",
        AgentIcon::OpenCode => "agent-open-code.svg",
    }
}
