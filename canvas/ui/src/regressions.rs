//! Regressions for the agent-connections feature.
//!
//! These guard the bugs fixed immediately before the feature (notice/FPS/
//! cleanup) and the rules it introduced (menu-driven edit dialog, edges from
//! admin-created cards, legacy configs). Test-only; never linked into the app.

use auri_protocol::{Request, Response};
use canvas_types::{
    AgentKind, CanvasState, CardBounds, ConnectionTarget, WorkspaceIcon, WorkspaceStore,
    WorkspaceState,
};

use crate::canvas::test_support;

fn bounds() -> CardBounds {
    CardBounds { x: 0.0, y: 0.0, width: 100.0, height: 100.0 }
}

#[test]
fn edit_dialog_opens_from_agent_menu() {
    let (mut menu, mut shell) = test_support::agent_menu_and_shell();
    assert!(!test_support::is_editing(&menu), "a menu click has not happened yet");
    assert!(test_support::has_dismiss_overlay(&shell), "the card's menu owns the overlay");

    test_support::click_edit(&mut menu, &mut shell);

    assert!(test_support::is_editing(&menu), "the edit form is active");
    assert!(
        !test_support::has_dismiss_overlay(&shell),
        "the menu's dismiss overlay must not cover the dialog"
    );
}

#[test]
fn workspace_notice_follows_load_state() {
    let mut store = WorkspaceStore::new();
    assert!(test_support::needs_workspace_notice(&store), "no active workspace");

    let workspace = store.add("A".into(), WorkspaceIcon::Star, std::env::temp_dir());
    assert!(test_support::needs_workspace_notice(&store), "the active workspace is unloaded");

    store.workspace_mut(workspace.id).unwrap().mark_loaded();
    assert!(!test_support::needs_workspace_notice(&store), "loading removes the notice");

    store.workspace_mut(workspace.id).unwrap().unload();
    assert!(test_support::needs_workspace_notice(&store), "unloading restores the notice");
}

#[test]
fn fps_badge_is_not_rendered() {
    let mut store = WorkspaceStore::new();
    let mut workspace = store.add("A".into(), WorkspaceIcon::Star, std::env::temp_dir());
    workspace.canvas.add_agent(AgentKind::Claude, bounds());

    let labels = test_support::render_state_labels(&store);
    assert!(!labels.is_empty(), "the canvas really produced labels");
    assert!(
        labels.iter().all(|label| !label.to_ascii_lowercase().contains("fps")),
        "no canvas label may expose an FPS badge: {labels:?}"
    );
}

#[test]
fn connection_dies_with_its_card() {
    let mut state = CanvasState::new();
    let card = state.add_agent(AgentKind::Claude, bounds()).id;
    let other = state.add_agent(AgentKind::Codex, bounds()).id;
    let note = state.add_note("oi".into(), bounds()).id;

    assert!(state.connect(card, ConnectionTarget::Card(other)));
    assert!(state.connect(other, ConnectionTarget::Note(note)));
    assert_eq!(state.connections().len(), 2);

    assert!(state.remove_agent(other).is_some());
    assert!(state.connections().is_empty(), "both edges ended with their card endpoint");
    assert!(!state.connected(card, ConnectionTarget::Card(other)));
    assert!(!state.connected(card, ConnectionTarget::Note(note)));

    assert!(state.connect(card, ConnectionTarget::Note(note)));
    assert!(state.remove_note(note).is_some());
    assert!(state.connections().is_empty(), "a note endpoint cleans up too");
}

#[test]
fn admin_created_card_is_connected_and_normal() {
    let mut harness = test_support::AdminHarness::new(true);
    let response = harness.run(Request::AgentNew { name: "Aux".into(), command: Some("pi".into()) });
    let Response::Ok(message) = &response else {
        panic!("expected a created agent, got {response:?}");
    };
    let id = message.split_whitespace().last().unwrap().parse::<u64>().unwrap();

    assert!(!harness.agent_admin(id), "admin-created agents are normal cards");
    assert!(harness.creator_is_admin(), "the creator keeps its role");
    assert!(harness.creator_connected_to(id), "the creator is connected to its new card");
}

/// A connected normal agent may work across workspaces; without the drawn
/// edge neither listing nor asking may reach the peer.
#[test]
fn connected_normal_peer_lists_and_asks_across_but_unconnected_is_refused() {
    let mut connected = test_support::PeerHarness::new(false, true);
    let Response::Ok(lines) = connected.run(Request::List) else {
        panic!("a normal agent must be able to list");
    };
    assert!(
        lines.lines().any(|line| line == "agent 2 Leitor normal @ auro"),
        "a connected remote endpoint is listed with its workspace: {lines}"
    );
    assert!(!lines.contains("Estranho"), "an unconnected workspace must not leak: {lines}");

    let reply = connected.run(Request::Ask {
        target: "Leitor @ Auro".into(),
        raw: b"vamos".to_vec(),
    });
    assert_eq!(reply, Response::Ok(String::new()));
    assert_eq!(connected.written(), "auri from Writer <1@auri>\nvamos\r");

    let mut unconnected = test_support::PeerHarness::new(false, false);
    let Response::Ok(lines) = unconnected.run(Request::List) else {
        panic!("listing is always allowed");
    };
    assert!(!lines.contains("Leitor"), "no edge means no remote address: {lines}");
    assert_eq!(
        unconnected.run(Request::Ask { target: "2@auro".into(), raw: b"vamos".to_vec() }),
        Response::Err("permission_denied".into())
    );
    assert!(unconnected.written().is_empty());
}

#[test]
fn old_config_regression_loads_without_connections() {
    // A config written before the feature: no `connections`, no `admin`,
    // no `unloaded` field. All of them must take their legacy defaults.
    let legacy = r#"{"id":1,"name":"AURI","icon":"Star","root":"/tmp","canvas":{"terminals":[],"notes":[],"agents":[{"id":1,"kind":"Claude","name":"Claude 1","bounds":{"x":12.0,"y":34.0,"width":640.0,"height":400.0}}],"selected":1,"next_id":2}}"#;
    let workspace: WorkspaceState = serde_json::from_str(legacy).expect("legacy workspace loads");

    assert!(workspace.canvas.connections().is_empty());
    assert!(workspace.canvas.agents().iter().all(|agent| !agent.admin));
    assert_eq!(workspace.canvas.selected(), Some(1));
    assert_eq!(workspace.canvas.agents()[0].bounds.width, 640.0);
}
