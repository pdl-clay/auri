//! Pure permission matrix for CLI requests from agent cards.
//!
//! The host resolves a request's token to a [`Principal`] and calls [`authorize`]
//! before any state changes. Authority comes from the stored [`AgentState::admin`]
//! bit and the workspace's user-drawn edges — never from a card's display name.

#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
use canvas_types::{AgentKind, CanvasState, WorkspaceIcon};

use canvas_types::{AgentState, ConnectionTarget, TerminalId, WorkspaceState};

/// A live agent card making a request. `admin` is the role snapshot taken when
/// the token was resolved; [`authorize`] re-reads the persisted role so a toggle
/// takes effect on the very next request, even with a stale snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Principal {
    pub id: TerminalId,
    pub admin: bool,
}

impl From<&AgentState> for Principal {
    fn from(agent: &AgentState) -> Self {
        Self {
            id: agent.id,
            admin: agent.admin,
        }
    }
}

/// One requested operation. `target` is a workspace-local id (or a note id),
/// never a display name. `raw` is the exact byte payload for `ask`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    List,
    Ask {
        target: ConnectionTarget,
        raw: Vec<u8>,
    },
    Check {
        target: ConnectionTarget,
    },
    NoteRead {
        note: u64,
    },
    NoteWrite {
        note: u64,
    },
    NoteCreate,
    NoteDelete {
        note: u64,
    },
    TerminalNew,
    TerminalRemove {
        target: ConnectionTarget,
    },
    AgentNew,
    AgentRemove {
        target: ConnectionTarget,
    },
    AgentRole {
        target: ConnectionTarget,
    },
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denied {
    NoIdentity,
    NotAdmin,
    NotConnected,
}

pub fn authorize(
    workspace: &WorkspaceState,
    principal: Option<&Principal>,
    action: &Action,
) -> Result<(), Denied> {
    let Some(principal) = principal else {
        return Err(Denied::NoIdentity);
    };

    // Only a live agent card in this workspace has an identity. Reading the
    // stored bit (rather than the caller's snapshot) makes a role toggle apply
    // to the next request, including demotion of an existing token.
    let admin = workspace
        .canvas
        .agent(principal.id)
        .ok_or(Denied::NoIdentity)?
        .admin;
    if admin {
        return Ok(());
    }

    let target = match action {
        Action::List => return Ok(()),
        Action::Ask { target, .. } | Action::Check { target } => *target,
        Action::NoteRead { note } | Action::NoteWrite { note } => ConnectionTarget::Note(*note),
        Action::NoteCreate
        | Action::NoteDelete { .. }
        | Action::TerminalNew
        | Action::TerminalRemove { .. }
        | Action::AgentNew
        | Action::AgentRemove { .. }
        | Action::AgentRole { .. } => return Err(Denied::NotAdmin),
    };

    if workspace.canvas.connected(principal.id, target) {
        Ok(())
    } else {
        Err(Denied::NotConnected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> canvas_types::CardBounds {
        canvas_types::CardBounds {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        }
    }

    fn workspace() -> WorkspaceState {
        WorkspaceState {
            id: 1,
            name: "Test".into(),
            icon: WorkspaceIcon::Star,
            root: PathBuf::from("/tmp/auri-permissions"),
            color: Default::default(),
            invalid_root: false,
            canvas: CanvasState::new(),
            unloaded: false,
        }
    }

    fn all_actions(note: u64, target: ConnectionTarget) -> Vec<Action> {
        vec![
            Action::List,
            Action::Ask {
                target,
                raw: Vec::new(),
            },
            Action::Check { target },
            Action::NoteRead { note },
            Action::NoteWrite { note },
            Action::NoteCreate,
            Action::NoteDelete { note },
            Action::TerminalNew,
            Action::TerminalRemove { target },
            Action::AgentNew,
            Action::AgentRemove { target },
            Action::AgentRole { target },
        ]
    }

    #[test]
    fn no_principal_is_no_identity_for_every_action() {
        let ws = workspace();
        let target = ConnectionTarget::Card(9);
        for action in all_actions(7, target) {
            assert_eq!(
                authorize(&ws, None, &action),
                Err(Denied::NoIdentity),
                "{action:?} without an identity"
            );
        }
    }

    #[test]
    fn normal_agent_with_note_edge_can_write_but_not_create() {
        let mut ws = workspace();
        let agent = ws.canvas.add_agent(AgentKind::Claude, bounds());
        let note = ws.canvas.add_note("hello".into(), bounds());
        assert!(ws.canvas.connect(agent.id, ConnectionTarget::Note(note.id)));
        let principal = Principal::from(ws.canvas.agent(agent.id).unwrap());

        assert_eq!(
            authorize(&ws, Some(&principal), &Action::NoteWrite { note: note.id }),
            Ok(())
        );
        assert_eq!(
            authorize(&ws, Some(&principal), &Action::NoteCreate),
            Err(Denied::NotAdmin)
        );
    }

    #[test]
    fn normal_agent_without_edge_cannot_ask() {
        let mut ws = workspace();
        let caller = ws.canvas.add_agent(AgentKind::Claude, bounds());
        let target = ws.canvas.add_agent(AgentKind::Codex, bounds());
        let principal = Principal::from(ws.canvas.agent(caller.id).unwrap());

        assert_eq!(
            authorize(
                &ws,
                Some(&principal),
                &Action::Ask {
                    target: ConnectionTarget::Card(target.id),
                    raw: b"hello".to_vec(),
                }
            ),
            Err(Denied::NotConnected)
        );
    }

    #[test]
    fn admin_can_reach_unconnected_targets() {
        let mut ws = workspace();
        let caller = ws.canvas.add_agent(AgentKind::Claude, bounds());
        let target = ws.canvas.add_agent(AgentKind::Codex, bounds());
        let note = ws.canvas.add_note("hello".into(), bounds());
        assert!(ws.canvas.set_agent_admin(caller.id, true));
        let principal = Principal::from(ws.canvas.agent(caller.id).unwrap());
        let card = ConnectionTarget::Card(target.id);

        assert_eq!(
            authorize(
                &ws,
                Some(&principal),
                &Action::Ask {
                    target: card,
                    raw: Vec::new(),
                }
            ),
            Ok(())
        );
        assert_eq!(
            authorize(&ws, Some(&principal), &Action::NoteWrite { note: note.id }),
            Ok(())
        );
        assert_eq!(
            authorize(
                &ws,
                Some(&principal),
                &Action::TerminalRemove { target: card }
            ),
            Ok(())
        );
        assert_eq!(
            authorize(&ws, Some(&principal), &Action::AgentRole { target: card }),
            Ok(())
        );
    }

    #[test]
    fn normal_agent_cannot_change_roles() {
        let mut ws = workspace();
        let caller = ws.canvas.add_agent(AgentKind::Claude, bounds());
        let target = ws.canvas.add_agent(AgentKind::Codex, bounds());
        let principal = Principal::from(ws.canvas.agent(caller.id).unwrap());

        assert_eq!(
            authorize(
                &ws,
                Some(&principal),
                &Action::AgentRole {
                    target: ConnectionTarget::Card(target.id),
                }
            ),
            Err(Denied::NotAdmin)
        );
    }

    #[test]
    fn role_toggle_takes_effect_on_next_request() {
        let mut ws = workspace();
        let caller = ws.canvas.add_agent(AgentKind::Claude, bounds());
        let principal = Principal::from(ws.canvas.agent(caller.id).unwrap());

        assert_eq!(
            authorize(&ws, Some(&principal), &Action::NoteCreate),
            Err(Denied::NotAdmin)
        );
        assert!(ws.canvas.set_agent_admin(caller.id, true));
        assert_eq!(
            authorize(&ws, Some(&principal), &Action::NoteCreate),
            Ok(())
        );
        assert!(ws.canvas.set_agent_admin(caller.id, false));
        assert_eq!(
            authorize(&ws, Some(&principal), &Action::NoteCreate),
            Err(Denied::NotAdmin)
        );
    }

    #[test]
    fn connected_normal_agent_can_use_all_five_edge_actions() {
        let mut ws = workspace();
        let caller = ws.canvas.add_agent(AgentKind::Claude, bounds());
        let target = ws.canvas.add_agent(AgentKind::Codex, bounds());
        let note = ws.canvas.add_note("hello".into(), bounds());
        // The card edge is drawn from the target: permission reads both directions.
        assert!(
            ws.canvas
                .connect(target.id, ConnectionTarget::Card(caller.id))
        );
        assert!(
            ws.canvas
                .connect(caller.id, ConnectionTarget::Note(note.id))
        );
        let principal = Principal::from(ws.canvas.agent(caller.id).unwrap());
        let card = ConnectionTarget::Card(target.id);

        for action in [
            Action::List,
            Action::Ask {
                target: card,
                raw: Vec::new(),
            },
            Action::Check { target: card },
            Action::NoteRead { note: note.id },
            Action::NoteWrite { note: note.id },
        ] {
            assert_eq!(
                authorize(&ws, Some(&principal), &action),
                Ok(()),
                "{action:?}"
            );
        }
    }

    #[test]
    fn note_edge_does_not_grant_access_to_a_card() {
        let mut ws = workspace();
        let caller = ws.canvas.add_agent(AgentKind::Claude, bounds());
        let target = ws.canvas.add_agent(AgentKind::Codex, bounds());
        let note = ws.canvas.add_note("hello".into(), bounds());
        assert!(
            ws.canvas
                .connect(caller.id, ConnectionTarget::Note(note.id))
        );
        let principal = Principal::from(ws.canvas.agent(caller.id).unwrap());

        assert_eq!(
            authorize(
                &ws,
                Some(&principal),
                &Action::Check {
                    target: ConnectionTarget::Card(target.id),
                }
            ),
            Err(Denied::NotConnected)
        );
        assert_eq!(
            authorize(&ws, Some(&principal), &Action::NoteRead { note: note.id }),
            Ok(())
        );
    }

    #[test]
    fn connected_normal_agent_is_denied_every_admin_action() {
        let mut ws = workspace();
        let caller = ws.canvas.add_agent(AgentKind::Claude, bounds());
        let target = ws.canvas.add_agent(AgentKind::Codex, bounds());
        let note = ws.canvas.add_note("hello".into(), bounds());
        assert!(
            ws.canvas
                .connect(caller.id, ConnectionTarget::Card(target.id))
        );
        assert!(
            ws.canvas
                .connect(caller.id, ConnectionTarget::Note(note.id))
        );
        let principal = Principal::from(ws.canvas.agent(caller.id).unwrap());
        let card = ConnectionTarget::Card(target.id);

        for action in [
            Action::NoteCreate,
            Action::NoteDelete { note: note.id },
            Action::TerminalNew,
            Action::TerminalRemove { target: card },
            Action::AgentNew,
            Action::AgentRemove { target: card },
            Action::AgentRole { target: card },
        ] {
            assert_eq!(
                authorize(&ws, Some(&principal), &action),
                Err(Denied::NotAdmin),
                "{action:?}"
            );
        }
    }

    #[test]
    fn admin_is_allowed_every_admin_action() {
        let mut ws = workspace();
        let caller = ws.canvas.add_agent(AgentKind::Claude, bounds());
        let target = ws.canvas.add_agent(AgentKind::Codex, bounds());
        let note = ws.canvas.add_note("hello".into(), bounds());
        assert!(ws.canvas.set_agent_admin(caller.id, true));
        let principal = Principal::from(ws.canvas.agent(caller.id).unwrap());
        let card = ConnectionTarget::Card(target.id);

        for action in [
            Action::NoteCreate,
            Action::NoteDelete { note: note.id },
            Action::TerminalNew,
            Action::TerminalRemove { target: card },
            Action::AgentNew,
            Action::AgentRemove { target: card },
            Action::AgentRole { target: card },
        ] {
            assert_eq!(
                authorize(&ws, Some(&principal), &action),
                Ok(()),
                "{action:?}"
            );
        }
    }

    #[test]
    fn display_name_and_plain_terminal_id_grant_no_authority() {
        let mut ws = workspace();
        let terminal = ws.canvas.add();
        let agent = ws
            .canvas
            .add_agent_named("administrator".into(), AgentKind::Claude, bounds());
        assert!(!ws.canvas.agent(agent.id).unwrap().admin);
        let normal = Principal::from(ws.canvas.agent(agent.id).unwrap());
        let forged = Principal {
            id: terminal.id,
            admin: true,
        };

        assert_eq!(
            authorize(&ws, Some(&normal), &Action::NoteCreate),
            Err(Denied::NotAdmin)
        );
        assert_eq!(
            authorize(&ws, Some(&forged), &Action::List),
            Err(Denied::NoIdentity)
        );
    }
}
