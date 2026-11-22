//! Framework-free types shared by the canvas host and its terminal cards.
//!
//! [`CanvasState`] tracks terminals, selection and id allocation; the host owns
//! the framework-specific entities separately.

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
}
