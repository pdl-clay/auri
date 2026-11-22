//! Canvas view skeleton; Task 5 adds rendering, cards and the toolbar.

use auro::gpui::Context;
use canvas_types::{CanvasState, TerminalId};

/// Owns the logical canvas state that terminal cards notify.
pub struct Canvas {
    pub(crate) state: CanvasState,
}

impl Canvas {
    pub fn new() -> Self {
        Self {
            state: CanvasState::new(),
        }
    }

    pub fn mark_exited(&mut self, id: TerminalId, cx: &mut Context<Self>) {
        self.state.mark_exited(id);
        cx.notify();
    }

    pub fn remove(&mut self, id: TerminalId) {
        self.state.remove(id);
    }
}
