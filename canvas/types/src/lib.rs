//! Types shared by the host view and the hot boundary.
//!
//! [`HotData`] is the ABI between a patched boundary and the host that is already running. Keep
//! its shape fixed: change [`ViewData`] and `HotData`'s encoder instead, bump [`HotData::SCHEMA`]
//! when the encoded fields change, and the old host rejects the new frame instead of reading it.

/// Bytes in the fixed frame. Changing this is an ABI change.
const FRAME_BYTES: usize = 240;
/// A title must fit here; a longer one is rejected instead of silently truncated.
const TITLE_BYTES: usize = 200;
/// Marks a frame whose payload did not fit; never a valid schema value.
const OVERFLOW: u64 = 1 << 63;

/// The user-facing view values. This type may change; the frame below carries them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewData {
    pub title: String,
    pub accent: u32,
}

/// Fixed-size frame returned by a hot boundary.
///
/// This is the one type a patch must not re-lay-out. It travels by value through the jump table,
/// so the running host and the patched function have to agree on its size and field offsets. The
/// rest of the view data can change freely: it is encoded into `bytes`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HotData {
    schema: u64,
    bytes: [u8; FRAME_BYTES],
}

impl HotData {
    /// Increment when the fields encoded into [`ViewData`] change.
    pub const SCHEMA: u64 = 1;
    /// Payload capacity. Large enough for a title and an accent; fixed is the point.
    pub const BYTES: usize = FRAME_BYTES;

    pub fn encode(data: &ViewData) -> Self {
        let mut bytes = [0u8; FRAME_BYTES];
        let overflow = data.title.len() > TITLE_BYTES;
        // An oversized title is rejected by `decode`. Do not slice it to the frame size: a
        // byte-sized cut can land inside a multi-byte UTF-8 character and panic in the boundary
        // before the host gets a chance to reject the frame.
        if !overflow {
            let title = data.title.as_bytes();
            bytes[..4].copy_from_slice(&(title.len() as u32).to_le_bytes());
            bytes[4..4 + title.len()].copy_from_slice(title);
        }
        let accent_at = 4 + TITLE_BYTES;
        bytes[accent_at..accent_at + 4].copy_from_slice(&data.accent.to_le_bytes());
        Self {
            schema: Self::SCHEMA | if overflow { OVERFLOW } else { 0 },
            bytes,
        }
    }

    pub fn decode(&self) -> Result<ViewData, &'static str> {
        if self.schema & OVERFLOW != 0 {
            return Err("the view title is longer than the fixed frame");
        }
        if self.schema != Self::SCHEMA {
            return Err("the patched boundary uses a different view-data schema");
        }
        let len = u32::from_le_bytes(self.bytes[..4].try_into().expect("four bytes")) as usize;
        if len > TITLE_BYTES {
            return Err("the patched boundary wrote a title longer than the frame");
        }
        let title = std::str::from_utf8(&self.bytes[4..4 + len])
            .map_err(|_| "the patched boundary wrote a title that is not UTF-8")?;
        let accent_at = 4 + TITLE_BYTES;
        let accent = u32::from_le_bytes(
            self.bytes[accent_at..accent_at + 4]
                .try_into()
                .expect("four bytes"),
        );
        Ok(ViewData {
            title: title.to_owned(),
            accent,
        })
    }
}

/// Identifier of a terminal on the canvas.
pub type TerminalId = u64;

/// Logical state of a single terminal card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalState {
    pub id: TerminalId,
    pub title: String,
    pub exited: bool,
}

/// Framework-free canvas state: terminals, the selected terminal and id allocation.
pub struct CanvasState {
    terminals: Vec<TerminalState>,
    selected: Option<TerminalId>,
    next_id: TerminalId,
}

impl CanvasState {
    pub fn new() -> Self {
        Self {
            terminals: Vec::new(),
            selected: None,
            next_id: 1,
        }
    }

    pub fn add(&mut self) -> TerminalState {
        let id = self.next_id;
        self.next_id += 1;
        let terminal = TerminalState {
            id,
            title: format!("Terminal {id}"),
            exited: false,
        };
        self.terminals.push(terminal.clone());
        self.selected = Some(id);
        terminal
    }

    pub fn remove(&mut self, id: TerminalId) -> Option<TerminalState> {
        let index = self.terminals.iter().position(|terminal| terminal.id == id)?;
        let removed = self.terminals.remove(index);
        if self.selected == Some(id) {
            self.selected = None;
        }
        Some(removed)
    }

    pub fn select(&mut self, id: TerminalId) {
        self.selected = Some(id);
    }

    pub fn selected(&self) -> Option<TerminalId> {
        self.selected
    }

    pub fn mark_exited(&mut self, id: TerminalId) {
        if let Some(terminal) = self.terminals.iter_mut().find(|terminal| terminal.id == id) {
            terminal.exited = true;
        }
    }

    pub fn terminals(&self) -> &[TerminalState] {
        &self.terminals
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_generates_sequential_ids_and_selects() {
        let mut state = CanvasState::new();
        let a = state.add();
        let b = state.add();
        assert_eq!((a.id, b.id), (1, 2));
        assert_eq!((a.title.as_str(), b.title.as_str()), ("Terminal 1", "Terminal 2"));
        assert_eq!(state.selected(), Some(2));
    }

    #[test]
    fn remove_clears_selection_only_when_removing_selected() {
        let mut state = CanvasState::new();
        let a = state.add();
        let _b = state.add();
        state.select(a.id);
        state.remove(a.id);
        assert_eq!(state.selected(), None);
        assert_eq!(state.terminals().len(), 1);
    }

    #[test]
    fn mark_exited_preserves_terminal_and_output_flag() {
        let mut state = CanvasState::new();
        let a = state.add();
        state.mark_exited(a.id);
        assert!(state.terminals()[0].exited);
        assert_eq!(state.terminals().len(), 1);
    }

    #[test]
    fn round_trips_the_view_data() {
        let data = ViewData {
            title: "Ready".to_owned(),
            accent: 0x123456,
        };
        assert_eq!(HotData::encode(&data).decode().expect("decodes"), data);
    }

    #[test]
    fn round_trips_a_full_multibyte_title() {
        let data = ViewData {
            title: "é".repeat(TITLE_BYTES / 2),
            accent: 0x123456,
        };
        assert_eq!(HotData::encode(&data).decode().expect("decodes"), data);
    }

    #[test]
    fn rejects_a_foreign_schema() {
        let mut frame = HotData::encode(&ViewData::default());
        frame.schema = HotData::SCHEMA + 1;
        assert!(frame.decode().is_err());
    }

    #[test]
    fn rejects_an_unaligned_oversized_title_without_panicking() {
        let data = ViewData {
            title: format!("a{}", "é".repeat(100)),
            accent: 0,
        };
        assert!(HotData::encode(&data).decode().is_err());
    }

    #[test]
    fn rejects_an_oversized_title_instead_of_truncating_it() {
        let data = ViewData {
            title: "x".repeat(TITLE_BYTES + 50),
            accent: 0,
        };
        assert!(HotData::encode(&data).decode().is_err());

        let data = ViewData {
            title: "é".repeat(TITLE_BYTES),
            accent: 0,
        };
        assert!(HotData::encode(&data).decode().is_err());
    }
}
