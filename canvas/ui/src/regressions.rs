//! Regressions for the agent-connections feature.
//!
//! These guard the bugs fixed immediately before the feature (notice/FPS/
//! cleanup) and the rules it introduced (menu-driven edit dialog, edges from
//! admin-created cards, legacy configs). Test-only; never linked into the app.

use auri_protocol::{Request, Response};
use canvas_types::{
    AgentKind, AgentView, CanvasState, CardBounds, ConnectionTarget, WorkspaceIcon, WorkspaceStore,
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
fn unloaded_workspace_hides_its_cards_and_ropes() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let mut store = cross_workspace_store(true);
    store.select(2);
    store.workspace_mut(2).unwrap().unload();
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.run_until_parked();

    assert!(test_support::needs_workspace_notice(&store));
    assert!(vcx.debug_bounds("proxy-card-1-c1").is_none());
    assert!(
        vcx.read(|cx| test_support::edge_click_point(view.read(cx), 0).is_none()),
        "an unloaded workspace must not paint a rope for a saved connection"
    );
}

#[test]
fn update_banner_appears_only_when_a_release_is_available() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = WorkspaceStore::new();
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("update-banner").is_none());

    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| canvas.set_update_available("0.1.2".into(), cx))
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("update-banner").is_some());
    assert!(vcx.debug_bounds("update-card-title").is_some());
    assert!(vcx.debug_bounds("update-card-detail").is_some());
}

#[test]
fn drawer_footer_reports_version_and_update_state() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = WorkspaceStore::new();
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("drawer-version").is_some());
    assert!(vcx.debug_bounds("drawer-update-status").is_none());

    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| canvas.set_update_available("0.1.4".into(), cx))
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("drawer-update-status").is_some());
}

#[test]
fn agent_view_toggle_persists() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let mut store = WorkspaceStore::new();
    let workspace = store.add("A".into(), WorkspaceIcon::Star, std::env::temp_dir());
    let agent = store
        .workspace_mut(workspace.id)
        .unwrap()
        .canvas
        .add_agent(AgentKind::Omp, bounds());
    store.workspace_mut(workspace.id).unwrap().mark_loaded();
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Loaded,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| {
            test_support::mount_agent_for_test(canvas, workspace.id, agent.id, cx);
            canvas.select(workspace.id, agent.id, cx);
            canvas.set_agent_view(workspace.id, agent.id, AgentView::Chat, cx);
        });
    });
    vcx.run_until_parked();

    let stored = vcx.update(|_, cx| {
        test_support::agent_view(view.read(cx), workspace.id, agent.id)
    });
    assert_eq!(stored, Some(AgentView::Chat));
    assert!(vcx.debug_bounds("terminal-toggle").is_some());
}

#[test]
fn clicking_the_view_toggle_does_not_reenter_the_card() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;
    use auro::gpui::Modifiers;

    let mut cx = cross_test_cx();
    let mut store = WorkspaceStore::new();
    let workspace = store.add("A".into(), WorkspaceIcon::Star, std::env::temp_dir());
    let agent = store
        .workspace_mut(workspace.id)
        .unwrap()
        .canvas
        .add_agent(AgentKind::Claude, bounds());
    store
        .workspace_mut(workspace.id)
        .unwrap()
        .canvas
        .set_agent_view(agent.id, AgentView::Chat);

    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Loaded,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.update(|window, cx| {
        view.update(cx, |canvas, cx| canvas.load_workspace(workspace.id, window, cx));
    });
    vcx.run_until_parked();

    let toggle = vcx.debug_bounds("terminal-toggle").expect("chat toggle");
    vcx.simulate_click(toggle.center(), Modifiers::none());
    vcx.run_until_parked();

    let stored = vcx.update(|_, cx| {
        test_support::agent_view(view.read(cx), workspace.id, agent.id)
    });
    assert_eq!(stored, Some(AgentView::Terminal));
    assert!(vcx.debug_bounds("chat-toggle").is_some());
}

#[test]
fn agent_view_reopens_saved_mode() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let mut store = WorkspaceStore::new();
    let workspace = store.add("A".into(), WorkspaceIcon::Star, std::env::temp_dir());
    let agent = store
        .workspace_mut(workspace.id)
        .unwrap()
        .canvas
        .add_agent(AgentKind::Claude, bounds());
    store
        .workspace_mut(workspace.id)
        .unwrap()
        .canvas
        .set_agent_view(agent.id, AgentView::Chat);
    assert_eq!(
        store.workspaces()[0].canvas.agents()[0].view,
        AgentView::Chat
    );

    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Loaded,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.update(|window, cx| {
        view.update(cx, |canvas, cx| {
            canvas.load_workspace(workspace.id, window, cx);
        });
    });
    vcx.run_until_parked();

    let live = vcx.update(|_, cx| {
        test_support::card_view(view.read(cx), workspace.id, agent.id, cx)
    });
    assert_eq!(live, Some(AgentView::Chat));
    assert!(vcx.debug_bounds("terminal-toggle").is_some());
    assert!(vcx.debug_bounds("terminal-body").is_some());
    assert!(vcx.debug_bounds("chat-view").is_some());
}

#[test]
fn chat_loads_omp_history_from_a_session_file() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let session_path = dir.path().join("session.jsonl");
    let mut file = std::fs::File::create(&session_path).unwrap();
    writeln!(file, r#"{{"type":"message","message":{{"role":"user","content":[{{"type":"text","text":"oi"}}]}}}}"#).unwrap();
    writeln!(file, r#"{{"type":"message","message":{{"role":"assistant","content":[{{"type":"text","text":"olá"}}]}}}}"#).unwrap();
    drop(file);

    let mut cx = cross_test_cx();
    let mut store = WorkspaceStore::new();
    let workspace = store.add("A".into(), WorkspaceIcon::Star, std::env::temp_dir());
    let agent = store
        .workspace_mut(workspace.id)
        .unwrap()
        .canvas
        .add_agent(AgentKind::Omp, bounds());
    store
        .workspace_mut(workspace.id)
        .unwrap()
        .canvas
        .set_agent_session(agent.id, session_path.to_string_lossy().into_owned());
    store
        .workspace_mut(workspace.id)
        .unwrap()
        .canvas
        .set_agent_view(agent.id, AgentView::Chat);
    store.workspace_mut(workspace.id).unwrap().mark_loaded();

    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Loaded,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| {
            test_support::mount_agent_for_test(canvas, workspace.id, agent.id, cx);
            canvas.select(workspace.id, agent.id, cx);
            cx.notify();
        });
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("chat-message-0").is_some());
    assert!(vcx.debug_bounds("chat-message-1").is_some());
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

    assert!(state.remove_agent(1, other).is_some());
    assert!(state.connections().is_empty(), "both edges ended with their card endpoint");
    assert!(!state.connected(1, card, ConnectionTarget::Card(other)));
    assert!(!state.connected(1, card, ConnectionTarget::Note(note)));

    assert!(state.connect(card, ConnectionTarget::Note(note)));
    assert!(state.remove_note(1, note).is_some());
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
    let ask_back = connected.run_as_peer(Request::Ask {
        target: "1@auri".into(),
        raw: b"resposta".to_vec(),
    });
    assert_eq!(ask_back, Response::Ok(String::new()));
    assert_eq!(connected.peer_written(), "auri from Leitor <2@auro>\nresposta\r");

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
    assert_eq!(
        unconnected.run_as_peer(Request::Ask { target: "1@auri".into(), raw: b"resposta".to_vec() }),
        Response::Err("permission_denied".into())
    );
    assert!(unconnected.peer_written().is_empty());
}

/// At startup a peer workspace has no live writers. A stale writer from an
/// earlier load must not receive bytes: the ask first rebuilds the peer and
/// registers its fresh writer, then writes the envelope to that writer.
#[test]
fn ask_to_unloaded_peer_waits_for_the_fresh_writer() {
    let probe = test_support::probe_unloaded_peer_ask(true);

    assert_eq!(probe.response, Response::Ok(String::new()));
    assert!(probe.has_peer_writer, "the fresh peer PTY writer must be registered");
    assert!(probe.stale_bytes.is_empty(), "bytes went to a stale writer: {:?}", probe.stale_bytes);
    assert_eq!(
        probe.fresh_bytes,
        "auri from Writer <1@auri>\nping\r",
        "the rebuilt peer writer must receive the exact envelope"
    );
    assert_eq!(probe.active_before, Some(1));
    assert_eq!(probe.active_after, Some(1), "loading a peer must not change the active workspace");
    assert!(!probe.peer_unloaded);
    assert_eq!(probe.peer_cards, 1);
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

/// The window header is a single-line strip. A previous pass set it to 64px,
/// which turned every card into a heavy framed panel; do not return to that.
#[test]
fn window_header_stays_a_compact_strip() {
    use crate::theme::{CLOSE_BUTTON_SIZE, CLOSE_ICON_SIZE, TITLE_HEIGHT};

    assert!(
        TITLE_HEIGHT <= 32.0,
        "the card header is a strip, not a row: {TITLE_HEIGHT}px"
    );
    assert!(
        CLOSE_BUTTON_SIZE + 8.0 <= TITLE_HEIGHT,
        "the close control needs 4px of air above and below inside the header"
    );
    assert!(
        CLOSE_ICON_SIZE < CLOSE_BUTTON_SIZE,
        "the × glyph must sit inside its hit target"
    );
}

/// Workspace rows are text until hovered. A previous pass gave every row a
/// filled panel, a border and 12px of air between them; keep the list tight.
#[test]
fn workspace_rows_stay_compact_and_chrome_free() {
    use crate::theme::{
        COLOR_PANEL, COLOR_ROW_BORDER_ACTIVE, COLOR_ROW_HOVER, WORKSPACE_ROW_GAP,
        WORKSPACE_ROW_PAD_Y,
    };

    assert_eq!(WORKSPACE_ROW_GAP, 0.0, "workspace rows must touch vertically");
    assert!(WORKSPACE_ROW_PAD_Y <= 3.0, "a row is text with minimal air: {WORKSPACE_ROW_PAD_Y}px");
    assert_ne!(COLOR_ROW_HOVER, COLOR_PANEL, "hover must lift the row off the drawer");
    assert_ne!(COLOR_ROW_BORDER_ACTIVE, 0, "the selected row keeps its subtle border");
}

/// The empty canvas is a quiet surface, not a bright grid. Keep the lines
/// faint so cards and ropes carry the contrast.
#[test]
fn canvas_grid_stays_translucent() {
    use crate::theme::{COLOR_GRID, COLOR_GRID_ALPHA};

    assert_ne!(COLOR_GRID, 0, "the grid still exists");
    assert!(
        (0.15..=0.25).contains(&COLOR_GRID_ALPHA),
        "grid lines should stay faint: {COLOR_GRID_ALPHA}"
    );
}

/// Regression: the same island press must work for terminal and agent cards.
/// The note test covers the UI-owned selection; this one covers the store's
/// card selection, with no PTY entity needed because the island only asks the
/// store which id is selected.
#[test]
fn island_connect_starts_from_a_selected_card() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;
    use auro::gpui::{Modifiers, TestAppContext};

    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::theme::Theme::change(
            gpui_component::theme::ThemeMode::Dark,
            None,
            cx,
        );
    });

    let mut store = WorkspaceStore::new();
    let workspace = store.add("Regressão".into(), WorkspaceIcon::Star, std::env::temp_dir());
    let card_id = {
        let workspace = store.workspace_mut(workspace.id).unwrap();
        workspace.mark_loaded();
        workspace.canvas.add_agent(AgentKind::Claude, bounds()).id
    };

    let (view, vcx) = cx.add_window_view(|window, cx| {
        let handle = window.window_handle();
        let mut canvas = Canvas::new(store.clone(), LoadOutcome::Fresh, handle, None, cx);
        canvas.select(workspace.id, card_id, cx);
        canvas
    });
    vcx.run_until_parked();

    let bolt = vcx
        .debug_bounds("island-connect")
        .expect("a selected card must expose the bolt island button");
    vcx.simulate_click(bolt.center(), Modifiers::none());
    vcx.run_until_parked();

    let (connecting_from, selected_note) =
        vcx.read(|cx| test_support::connect_state(view.read(cx)));
    assert_eq!(
        connecting_from,
        Some(ConnectionTarget::Card(card_id)),
        "the bolt press must enter connection mode with the card as source"
    );
    assert_eq!(selected_note, None, "a card source never paints as a note");
}

/// Regression: clicking a note (or a card) and then the island's bolt cleared
/// the selection on mouse-down, so the click handler saw no source, returned,
/// and the bolt vanished. Real GPUI dispatch: select a note, press the bolt.
#[test]
fn island_connect_keeps_the_note_selection_and_starts_the_gesture() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;
    use auro::gpui::{Modifiers, TestAppContext};

    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::theme::Theme::change(
            gpui_component::theme::ThemeMode::Dark,
            None,
            cx,
        );
    });

    let mut store = WorkspaceStore::new();
    let workspace = store.add("Regressão".into(), WorkspaceIcon::Star, std::env::temp_dir());
    let note_id = {
        let workspace = store.workspace_mut(workspace.id).unwrap();
        workspace.mark_loaded();
        workspace
            .canvas
            .add_note("origem".into(), bounds())
            .id
    };

    let (view, vcx) = cx.add_window_view(|window, cx| {
        let handle = window.window_handle();
        let mut canvas = Canvas::new(store.clone(), LoadOutcome::Fresh, handle, None, cx);
        canvas.mount_note_for_test(workspace.id, note_id, window, cx);
        canvas
    });
    vcx.run_until_parked();

    let bolt = vcx
        .debug_bounds("island-connect")
        .expect("a selected note must expose the bolt island button");
    vcx.simulate_click(bolt.center(), Modifiers::none());
    vcx.run_until_parked();

    let (connecting_from, selected_note) = vcx.read(|cx| test_support::connect_state(view.read(cx)));
    assert_eq!(
        connecting_from,
        Some(ConnectionTarget::Note(note_id)),
        "the bolt press must enter connection mode with the note as source"
    );
    assert_eq!(
        selected_note,
        Some((workspace.id, note_id)),
        "the bolt press must not clear the selection before its click handler runs"
    );
}

/// The click target must follow the painted curve, not its bounding box.
#[test]
fn rope_hit_test_follows_the_curve() {
    let edges = [test_support::edge_hit(
        7,
        (0.0, 0.0),
        (100.0, 100.0),
        (200.0, 0.0),
    )];
    assert_eq!(
        test_support::edge_hit_at(&edges, 100.0, 50.0),
        Some(7),
        "the midpoint of the quadratic curve is on the rope"
    );
    assert_eq!(
        test_support::edge_hit_at(&edges, 100.0, 92.0),
        None,
        "empty space above the rope must not open the inspector"
    );
}

/// Pressing a connection opens the inspector; its remove button deletes that
/// edge and only that edge. Real GPUI dispatch through the canvas background.
#[test]
fn pressing_a_connection_opens_the_inspector_and_removes_it() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;
    use auro::gpui::{Modifiers, Point, TestAppContext, px};

    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::theme::Theme::change(
            gpui_component::theme::ThemeMode::Dark,
            None,
            cx,
        );
    });

    let mut store = WorkspaceStore::new();
    let workspace = store.add("Regressão".into(), WorkspaceIcon::Star, std::env::temp_dir());
    let (first, second) = {
        let workspace = store.workspace_mut(workspace.id).unwrap();
        workspace.mark_loaded();
        let first = workspace
            .canvas
            .add_note(
                "origem".into(),
                CardBounds { x: 40.0, y: 40.0, width: 220.0, height: 120.0 },
            )
            .id;
        let second = workspace
            .canvas
            .add_note(
                "destino".into(),
                CardBounds { x: 400.0, y: 340.0, width: 220.0, height: 120.0 },
            )
            .id;
        workspace.canvas.connect_note(first, ConnectionTarget::Note(second));
        (first, second)
    };

    let (view, vcx) = cx.add_window_view(|window, cx| {
        let handle = window.window_handle();
        let mut canvas = Canvas::new(store.clone(), LoadOutcome::Fresh, handle, None, cx);
        canvas.mount_note_for_test(workspace.id, first, window, cx);
        canvas.mount_note_for_test(workspace.id, second, window, cx);
        canvas
    });
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| test_support::connection_count(view.read(cx))), 1);

    let (x, y, drawer_width) = vcx.read(|cx| {
        let canvas = view.read(cx);
        let point = test_support::edge_click_point(canvas, 0).expect("painted rope");
        (point.0, point.1, test_support::drawer_width(canvas, cx))
    });
    vcx.simulate_click(Point::new(px(x + drawer_width), px(y)), Modifiers::none());
    vcx.run_until_parked();
    assert!(
        vcx.read(|cx| test_support::connection_popover_open(view.read(cx))),
        "pressing the rope opens the connection inspector"
    );
    assert!(vcx.debug_bounds("connection-popover").is_some());
    assert!(vcx.debug_bounds("connection-remove").is_some());

    let remove = vcx
        .debug_bounds("connection-remove")
        .expect("remove control inside the inspector");
    vcx.simulate_click(remove.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        vcx.read(|cx| test_support::connection_count(view.read(cx))),
        0,
        "the inspector removes the pressed connection"
    );
    assert!(
        !vcx.read(|cx| test_support::connection_popover_open(view.read(cx))),
        "the inspector closes after removal"
    );
}

/// The note's right-click menu is a second entry point to the same gesture as
/// the island bolt. Real dispatch: right-click the note, click `Conectar`.
#[test]
fn right_click_menu_starts_connection_from_note() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;
    use auro::gpui::{Modifiers, MouseButton, Point, TestAppContext, px};

    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::theme::Theme::change(
            gpui_component::theme::ThemeMode::Dark,
            None,
            cx,
        );
    });

    let mut store = WorkspaceStore::new();
    let workspace = store.add("Regressão".into(), WorkspaceIcon::Star, std::env::temp_dir());
    let note_id = {
        let workspace = store.workspace_mut(workspace.id).unwrap();
        workspace.mark_loaded();
        workspace.canvas.add_note("origem".into(), bounds()).id
    };

    let (view, vcx) = cx.add_window_view(|window, cx| {
        let handle = window.window_handle();
        let mut canvas = Canvas::new(store.clone(), LoadOutcome::Fresh, handle, None, cx);
        canvas.mount_note_for_test(workspace.id, note_id, window, cx);
        canvas
    });
    vcx.run_until_parked();

    let drawer_width = vcx.read(|cx| test_support::drawer_width(view.read(cx), cx));
    vcx.simulate_mouse_down(
        Point::new(px(drawer_width + 50.0), px(16.0)),
        MouseButton::Right,
        Modifiers::none(),
    );
    vcx.run_until_parked();
    let item = vcx
        .debug_bounds("connect-menu-item")
        .expect("right-click on a note offers Conectar");
    vcx.simulate_click(item.center(), Modifiers::none());
    vcx.run_until_parked();
    let (connecting_from, _) = vcx.read(|cx| test_support::connect_state(view.read(cx)));
    assert_eq!(
        connecting_from,
        Some(ConnectionTarget::Note(note_id)),
        "the menu's Conectar item must start the same gesture as the island bolt"
    );
}


#[cfg(test)]
struct ScrollProbe {
    terminal: auro::gpui::Entity<gpui_terminal::TerminalView>,
    wheels: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl auro::gpui::Render for ScrollProbe {
    fn render(
        &mut self,
        _window: &mut auro::gpui::Window,
        _cx: &mut auro::gpui::Context<Self>,
    ) -> impl auro::gpui::IntoElement {
        use auro::gpui::{InteractiveElement, ParentElement, Styled, div};
        let counted = self.wheels.clone();
        div()
            .size_full()
            .on_scroll_wheel(move |_, _, cx| {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                cx.stop_propagation();
            })
            .child(self.terminal.clone())
    }
}
/// Wheel over a terminal must scroll its history and must not reach the
/// canvas's pan handler sitting on an ancestor.
#[test]
fn wheel_over_terminal_scrolls_history_not_canvas() {
    use auro::gpui::{AppContext, Modifiers, Point, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, px};
    use gpui_terminal::{TerminalConfig, TerminalView};
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut cx = TestAppContext::single();
    let mut bytes = Vec::new();
    for line in 0..400 {
        bytes.extend_from_slice(format!("linha {line}\r\n").as_bytes());
    }
    let terminal = cx.new(|cx| {
        TerminalView::new(Vec::new(), Cursor::new(bytes), TerminalConfig::default(), cx)
    });
    let ancestor_wheels = Arc::new(AtomicUsize::new(0));
    let (_view, vcx) = cx.add_window_view(|_, _| ScrollProbe {
        terminal: terminal.clone(),
        wheels: ancestor_wheels.clone(),
    });
    vcx.run_until_parked();

    vcx.simulate_click(Point::new(px(120.0), px(120.0)), Modifiers::none());
    vcx.run_until_parked();
    let before = vcx.read(|cx| terminal.read(cx).scroll_info());
    vcx.simulate_event(ScrollWheelEvent {
        position: Point::new(px(120.0), px(120.0)),
        delta: ScrollDelta::Lines(Point::new(0.0, 1.0)),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::default(),
    });
    vcx.run_until_parked();
    let after = vcx.read(|cx| terminal.read(cx).scroll_info());

    assert!(after.0 > before.0, "wheel down moves into scrollback: {before:?} -> {after:?}");
    assert_eq!(
        ancestor_wheels.load(Ordering::SeqCst),
        0,
        "the terminal must stop the wheel before an ancestor pans the canvas"
    );
}

/// A wheel event that cannot move the terminal (no history) must keep
/// propagating: swallowing it leaves the enclosing scroll area frozen.
#[test]
fn wheel_without_history_reaches_the_enclosing_scroll_area() {
    use auro::gpui::{AppContext, Modifiers, Point, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, px};

    let mut cx = TestAppContext::single();
    let terminal = cx.new(|cx| {
        gpui_terminal::TerminalView::new(
            Vec::new(),
            std::io::Cursor::new(Vec::new()),
            gpui_terminal::TerminalConfig::default(),
            cx,
        )
    });
    let ancestor_wheels = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (_view, vcx) = cx.add_window_view(|_, _| ScrollProbe {
        terminal: terminal.clone(),
        wheels: ancestor_wheels.clone(),
    });
    vcx.run_until_parked();
    vcx.simulate_event(ScrollWheelEvent {
        position: Point::new(px(120.0), px(120.0)),
        delta: ScrollDelta::Lines(Point::new(0.0, -1.0)),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::default(),
    });
    vcx.run_until_parked();

    assert_eq!(
        ancestor_wheels.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "an unconsumed wheel must reach the scroll area behind the terminal"
    );
}

/// One wheel notch is three lines regardless of canvas zoom. Converting a
/// line delta through a fixed 16px budget rounds it to zero at large fonts,
/// which is the frozen-scroll symptom at high zoom.
#[test]
fn wheel_lines_scroll_history_with_a_large_font() {
    use auro::gpui::{AppContext, Modifiers, Point, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, px};

    let mut cx = TestAppContext::single();
    let mut bytes = Vec::new();
    for line in 0..400 {
        bytes.extend_from_slice(format!("linha {line}\r\n").as_bytes());
    }
    let config = gpui_terminal::TerminalConfig {
        font_size: px(64.0),
        line_height_multiplier: 2.0,
        ..gpui_terminal::TerminalConfig::default()
    };
    let terminal = cx.new(|cx| {
        gpui_terminal::TerminalView::new(Vec::new(), std::io::Cursor::new(bytes), config, cx)
    });
    let (_view, vcx) = cx.add_window_view(|_, _| ScrollProbe {
        terminal: terminal.clone(),
        wheels: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });
    let mut history_ready = false;
    for _ in 0..50 {
        vcx.run_until_parked();
        if vcx.read(|cx| terminal.read(cx).scroll_info()).1 > 0 {
            history_ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(history_ready, "the async PTY reader must load the history first");
    let before = vcx.read(|cx| terminal.read(cx).scroll_info());
    vcx.simulate_event(ScrollWheelEvent {
        position: Point::new(px(120.0), px(120.0)),
        delta: ScrollDelta::Lines(Point::new(0.0, 1.0)),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::default(),
    });
    vcx.run_until_parked();
    let after = vcx.read(|cx| terminal.read(cx).scroll_info());

    assert!(after.0 > before.0, "one wheel notch must scroll history: {before:?} -> {after:?}");
}

#[test]
fn wheel_down_pans_canvas_content_down() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;
    use auro::gpui::{Point, px};

    let mut cx = cross_test_cx();
    let mut store = WorkspaceStore::new();
    let workspace = store.add("A".into(), WorkspaceIcon::Star, std::env::temp_dir());
    store.workspace_mut(workspace.id).unwrap().mark_loaded();
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.run_until_parked();

    let before = vcx.read(|cx| test_support::vertical_offset(view.read(cx)));
    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| {
            canvas.apply_scroll(Point::new(px(400.0), px(200.0)), 0.0, 16.0, false, false, cx)
        });
    });
    let after_down = vcx.read(|cx| test_support::vertical_offset(view.read(cx)));
    assert!(after_down > before, "wheel down must move cards down: {before} -> {after_down}");

    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| {
            canvas.apply_scroll(Point::new(px(400.0), px(200.0)), 0.0, -16.0, false, false, cx)
        });
    });
    let after_up = vcx.read(|cx| test_support::vertical_offset(view.read(cx)));
    assert!(after_up < after_down, "wheel up must move cards up: {after_down} -> {after_up}");
}

/// Placement must honour the click in the upper half of the canvas. A previous
/// pass always fell back to the "next to the last card" origin, pinning new
/// cards to the lower half even when the click was near the top.
#[test]
fn placement_click_in_the_upper_half_spawns_the_card_there() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;
    use auro::gpui::{Modifiers, MouseButton, MouseDownEvent, Point, TestAppContext, px};

    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::theme::Theme::change(
            gpui_component::theme::ThemeMode::Dark,
            None,
            cx,
        );
    });
    let mut store = WorkspaceStore::new();
    let workspace = store.add("Regressão".into(), WorkspaceIcon::Star, std::env::temp_dir());
    store.workspace_mut(workspace.id).unwrap().mark_loaded();

    let (view, vcx) = cx.add_window_view(|window, cx| {
        let mut canvas = Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        );
        // Wheel down moves the canvas content down: offset_y becomes +600.
        canvas.apply_scroll(Point::new(px(400.0), px(200.0)), 0.0, 600.0, false, false, cx);
        canvas.begin_placement(cx);
        canvas
    });
    vcx.run_until_parked();
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: Point::new(px(400.0), px(120.0)),
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: true,
    });
    vcx.run_until_parked();

    let placed = vcx.read(|cx| test_support::terminal_bounds(view.read(cx), workspace.id, cx));
    let placed = placed.expect("the placement click must spawn a terminal");
    let expected_y = 120.0 - 600.0 - crate::theme::CARD_HEIGHT / 2.0;
    assert!(
        (placed.y - expected_y).abs() < 1.0,
        "a click at y=120 on a canvas panned down must centre the card there \
         (world y {expected_y}), not clamp it into the lower half: {placed:?}"
    );

    // The note path has its own placement function; it must use the same
    // world-coordinate origin instead of clamping back into the lower half.
    vcx.update(|_, cx| view.update(cx, |canvas, cx| canvas.begin_text_placement(cx)));
    vcx.run_until_parked();
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: Point::new(px(400.0), px(120.0)),
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: true,
    });
    vcx.run_until_parked();
    let note = vcx
        .read(|cx| test_support::note_bounds(view.read(cx), workspace.id, cx))
        .expect("the placement click must spawn a note");
    let expected_note_y = 120.0 - 600.0 - crate::theme::TEXT_CARD_HEIGHT / 2.0;
    assert!(
        (note.y - expected_note_y).abs() < 1.0,
        "a note placed at y=120 must use the clicked world position: {note:?}"
    );
}

/// New terminals and agents open as a usable workspace, not a thumbnail.
#[test]
fn spawned_cards_open_large() {
    use crate::theme::{CARD_HEIGHT, CARD_WIDTH};

    assert!(CARD_WIDTH >= 900.0, "a new terminal needs room to work: {CARD_WIDTH}px");
    assert!(CARD_HEIGHT >= 540.0, "a new terminal needs room to read: {CARD_HEIGHT}px");
}

// --- Cross-workspace gesture, proxy card and rope removal -------------------

fn cross_test_cx() -> auro::gpui::TestAppContext {
    use auro::gpui::TestAppContext;

    let cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::theme::Theme::change(
            gpui_component::theme::ThemeMode::Dark,
            None,
            cx,
        );
    });
    cx
}

/// Auri owns an admin agent and a normal agent; Auro owns card 4. Both
/// workspaces deliberately use the same id space for unrelated cards.
fn cross_workspace_store(with_edge: bool) -> WorkspaceStore {
    let json = r#"{
        "workspaces": [
            {"id":1,"name":"Auri","icon":"Star","root":"/tmp","canvas":{
                "agents":[
                    {"id":1,"kind":"Claude","name":"Writer","bounds":{"x":40.0,"y":40.0,"width":240.0,"height":120.0},"admin":true},
                    {"id":2,"kind":"Codex","name":"Helper","bounds":{"x":40.0,"y":220.0,"width":240.0,"height":120.0},"admin":false}
                ],"terminals":[],"notes":[],"selected":null,"next_id":3,"connections":[]}},
            {"id":2,"name":"Auro","icon":"Folder","root":"/tmp","canvas":{
                "agents":[
                    {"id":4,"kind":"Pi","name":"Oh My Pi #3","bounds":{"x":600.0,"y":300.0,"width":240.0,"height":120.0},"admin":false}
                ],"terminals":[],"notes":[],"selected":null,"next_id":5,"connections":[]}}
        ],"active":1,"next_id":5
    }"#;
    let json = if with_edge {
        json.replacen(
            "\"connections\":[]",
            r#""connections":[{"from":{"Card":1},"to":{"Card":4},"from_workspace":1,"to_workspace":2}]"#,
            1,
        )
    } else {
        json.to_owned()
    };
    serde_json::from_str(&json).expect("cross-workspace fixture")
}

/// The human owns the gesture: any selected card, admin or not, can follow the
/// drawer into another workspace and become a cross-workspace source.
#[test]
fn cross_workspace_gesture_is_available_to_any_selected_card() {
    use auro::gpui::Modifiers;

    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = cross_workspace_store(false);
    let (view, vcx) = cx.add_window_view(|window, cx| {
        let mut canvas = Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        );
        canvas.select(1, 2, cx); // "Helper" is a normal agent
        canvas
    });
    vcx.run_until_parked();

    let bolt = vcx
        .debug_bounds("island-connect")
        .expect("a selected card exposes the bolt");
    vcx.simulate_click(bolt.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        vcx.read(|cx| test_support::connect_state(view.read(cx)).0),
        Some(ConnectionTarget::Card(2))
    );
    assert!(
        vcx.read(|cx| test_support::cross_connect_open(view.read(cx))),
        "a normal agent can start a cross-workspace gesture"
    );
    assert!(
        vcx.debug_bounds("connection-workspace-picker").is_none(),
        "the drawer is the only workspace selector"
    );

    vcx.update(|window, cx| {
        view.update(cx, |canvas, cx| {
            test_support::activate_via_drawer(canvas, 2, window, cx)
        })
    });
    vcx.run_until_parked();
    assert!(
        vcx.read(|cx| test_support::cross_connect_open(view.read(cx))),
        "switching workspace must keep the gesture alive for a normal agent"
    );

    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| {
            canvas.complete_connect(ConnectionTarget::Card(4), cx)
        })
    });
    vcx.run_until_parked();
    assert!(
        vcx.read(|cx| test_support::cross_connected(
            view.read(cx),
            1,
            ConnectionTarget::Card(2),
            2,
            ConnectionTarget::Card(4)
        )),
        "the click must store a workspace-qualified edge from the normal agent"
    );
    assert_eq!(vcx.read(|cx| test_support::cross_edge_count(view.read(cx))), 1);
}

#[test]
fn proxies_with_a_shared_numeric_id_are_separate_elements() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let json = r#"{
        "workspaces": [
            {"id":1,"name":"Auri","icon":"Star","root":"/tmp","canvas":{
                "agents":[{"id":1,"kind":"Claude","name":"Writer",
                    "bounds":{"x":40.0,"y":40.0,"width":240.0,"height":120.0},"admin":true}],
                "connections":[
                    {"from":{"Card":1},"to":{"Card":4},"from_workspace":1,"to_workspace":2},
                    {"from":{"Card":1},"to":{"Note":4},"from_workspace":1,"to_workspace":2}
                ],"next_id":2}},
            {"id":2,"name":"Auro","icon":"Folder","root":"/tmp","canvas":{
                "agents":[{"id":4,"kind":"Pi","name":"Oh My Pi #3",
                    "bounds":{"x":600.0,"y":300.0,"width":240.0,"height":120.0}}],
                "notes":[{"id":4,"text":"peer note",
                    "bounds":{"x":600.0,"y":520.0,"width":240.0,"height":120.0}}],
                "next_id":5}}
        ],"active":1,"next_id":3
    }"#;
    let store: WorkspaceStore = serde_json::from_str(json).expect("two proxies fixture");
    let mut cx = cross_test_cx();
    let (_, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(store.clone(), LoadOutcome::Fresh, window.window_handle(), None, cx)
    });
    vcx.run_until_parked();

    let card = vcx.debug_bounds("proxy-card-2-c4").expect("the card proxy");
    let note = vcx.debug_bounds("proxy-card-2-n4").expect("the note proxy");
    assert_ne!(card.center(), note.center(), "both endpoints must render at their own bounds");
}

#[test]
fn switching_workspace_closes_the_connection_popover() {
    use auro::gpui::{Modifiers, Point, px};

    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = cross_workspace_store(true);
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(store.clone(), LoadOutcome::Fresh, window.window_handle(), None, cx)
    });
    vcx.run_until_parked();
    let (x, y, drawer_width) = vcx.read(|cx| {
        let canvas = view.read(cx);
        let point = test_support::edge_click_point(canvas, 0).expect("the cross rope is painted");
        (point.0, point.1, test_support::drawer_width(canvas, cx))
    });
    vcx.simulate_click(Point::new(px(x + drawer_width), px(y)), Modifiers::none());
    vcx.run_until_parked();
    assert!(
        vcx.read(|cx| test_support::connection_popover_open(view.read(cx))),
        "the rope inspector is open in Auri"
    );

    vcx.update(|window, cx| {
        view.update(cx, |canvas, cx| {
            test_support::activate_via_drawer(canvas, 2, window, cx)
        })
    });
    vcx.run_until_parked();
    assert!(
        !vcx.read(|cx| test_support::connection_popover_open(view.read(cx))),
        "an inspector belongs to the workspace that opened it"
    );
}

/// Review Focus: a proxy is a representation, not a control. Pressing it must
/// not select, open the inspector, start a gesture or mutate the edge.
#[test]
fn clicking_a_proxy_card_changes_nothing() {
    use auro::gpui::Modifiers;

    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = cross_workspace_store(true);
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.run_until_parked();

    let proxy = vcx
        .debug_bounds("proxy-card-2-c4")
        .expect("a saved cross edge renders a proxy in Auri");
    let before = vcx.read(|cx| {
        let canvas = view.read(cx);
        (
            test_support::selection_state(canvas),
            test_support::connect_state(canvas),
            test_support::connection_popover_open(canvas),
            test_support::cross_connect_open(canvas),
            test_support::cross_edge_count(canvas),
        )
    });
    vcx.simulate_click(proxy.center(), Modifiers::none());
    vcx.run_until_parked();
    let after = vcx.read(|cx| {
        let canvas = view.read(cx);
        (
            test_support::selection_state(canvas),
            test_support::connect_state(canvas),
            test_support::connection_popover_open(canvas),
            test_support::cross_connect_open(canvas),
            test_support::cross_edge_count(canvas),
        )
    });
    assert_eq!(before, after, "the proxy must be completely inert");
    assert!(vcx.debug_bounds("connection-popover").is_none());
}

/// The same stored edge renders a proxy on both sides, and the existing rope
/// inspector removes it for both workspaces.
#[test]
fn cross_rope_popover_removes_the_edge_from_both_workspaces() {
    use auro::gpui::{Modifiers, Point, px};

    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = cross_workspace_store(true);
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.run_until_parked();

    assert!(
        vcx.debug_bounds("proxy-card-2-c4").is_some(),
        "Auri shows Auro's endpoint"
    );
    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| test_support::activate_workspace(canvas, 2, cx))
    });
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("proxy-card-1-c1").is_some(),
        "Auro shows the originating endpoint from the same edge"
    );
    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| test_support::activate_workspace(canvas, 1, cx))
    });
    vcx.run_until_parked();

    let (x, y, drawer_width) = vcx.read(|cx| {
        let canvas = view.read(cx);
        let point = test_support::edge_click_point(canvas, 0).expect("the cross rope is painted");
        (point.0, point.1, test_support::drawer_width(canvas, cx))
    });
    vcx.simulate_click(Point::new(px(x + drawer_width), px(y)), Modifiers::none());
    vcx.run_until_parked();
    assert!(
        vcx.read(|cx| test_support::connection_popover_open(view.read(cx))),
        "a cross rope opens the same inspector as a local one"
    );
    let remove = vcx
        .debug_bounds("connection-remove")
        .expect("the inspector offers removal");
    vcx.simulate_click(remove.center(), Modifiers::none());
    vcx.run_until_parked();

    assert!(
        !vcx.read(|cx| test_support::cross_connected(
            view.read(cx),
            1,
            ConnectionTarget::Card(1),
            2,
            ConnectionTarget::Card(4)
        )),
        "removal deletes the shared edge"
    );
    assert_eq!(vcx.read(|cx| test_support::cross_edge_count(view.read(cx))), 0);
    assert_eq!(vcx.read(|cx| test_support::proxy_count(view.read(cx))), 0);
    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| test_support::activate_workspace(canvas, 2, cx))
    });
    vcx.run_until_parked();
    assert_eq!(
        vcx.read(|cx| test_support::proxy_count(view.read(cx))),
        0,
        "the peer's proxy disappears too"
    );
}

/// A local edge is stored before the cross edge, so removing it while the
/// inspector is open shifts any index-based identity by one.
fn edge_order_store() -> WorkspaceStore {
    let json = r#"{
        "workspaces": [
            {"id":1,"name":"Auri","icon":"Star","root":"/tmp","canvas":{
                "agents":[
                    {"id":1,"kind":"Claude","name":"Writer","bounds":{"x":40.0,"y":40.0,"width":240.0,"height":120.0},"admin":true},
                    {"id":2,"kind":"Codex","name":"Helper","bounds":{"x":40.0,"y":260.0,"width":240.0,"height":120.0},"admin":false}
                ],"terminals":[],"notes":[],"selected":null,"next_id":3,
                "connections":[
                    {"from":{"Card":1},"to":{"Card":2}},
                    {"from":{"Card":1},"to":{"Card":4},"from_workspace":1,"to_workspace":2}
                ]}},
            {"id":2,"name":"Auro","icon":"Folder","root":"/tmp","canvas":{
                "agents":[{"id":4,"kind":"Pi","name":"Peer","bounds":{"x":600.0,"y":300.0,"width":240.0,"height":120.0}}],
                "connections":[]}}
        ],"active":1
    }"#;
    serde_json::from_str(json).expect("edge-order fixture")
}

/// A remote note sits exactly on top of a real local note.
fn overlapping_proxy_store() -> WorkspaceStore {
    let json = r#"{
        "workspaces": [
            {"id":1,"name":"Auri","icon":"Star","root":"/tmp","canvas":{
                "agents":[{"id":1,"kind":"Claude","name":"Writer","bounds":{"x":40.0,"y":40.0,"width":240.0,"height":120.0},"admin":true}],
                "terminals":[],
                "notes":[{"id":5,"text":"local note","bounds":{"x":600.0,"y":300.0,"width":240.0,"height":120.0}}],
                "connections":[{"from":{"Card":1},"to":{"Note":7},"from_workspace":1,"to_workspace":2}]}},
            {"id":2,"name":"Auro","icon":"Folder","root":"/tmp","canvas":{
                "agents":[],"terminals":[],
                "notes":[{"id":7,"text":"remote note","bounds":{"x":600.0,"y":300.0,"width":240.0,"height":120.0}}]}}
        ],"active":1
    }"#;
    serde_json::from_str(json).expect("overlapping-proxy fixture")
}

/// Activating the target workspace from the drawer is the workspace step: the
/// gesture stays alive without rendering a second workspace menu.
#[test]
fn drawer_activation_keeps_the_cross_gesture_alive() {
    use auro::gpui::Modifiers;

    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = cross_workspace_store(false);
    let (view, vcx) = cx.add_window_view(|window, cx| {
        let mut canvas = Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        );
        canvas.select(1, 1, cx);
        canvas
    });
    vcx.run_until_parked();
    let bolt = vcx.debug_bounds("island-connect").expect("admin bolt");
    vcx.simulate_click(bolt.center(), Modifiers::none());
    vcx.run_until_parked();

    vcx.update(|window, cx| {
        view.update(cx, |canvas, cx| {
            test_support::activate_via_drawer(canvas, 2, window, cx)
        })
    });
    vcx.run_until_parked();
    assert!(
        vcx.read(|cx| test_support::cross_connect_open(view.read(cx))),
        "activation must not close the cross-workspace gesture"
    );
    assert!(
        vcx.debug_bounds("connection-workspace-picker").is_none(),
        "the drawer is the only workspace selector"
    );

    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| {
            canvas.complete_connect(ConnectionTarget::Card(4), cx)
        })
    });
    vcx.run_until_parked();
    assert!(
        vcx.read(|cx| test_support::cross_connected(
            view.read(cx),
            1,
            ConnectionTarget::Card(1),
            2,
            ConnectionTarget::Card(4)
        )),
        "the source stays in its original workspace while the target is the activated one"
    );
    assert_eq!(vcx.read(|cx| test_support::cross_edge_count(view.read(cx))), 1);
}

/// The inspector must remove the edge it displayed, even if a store change
/// reorders or rebuilds the painted edge list while it is open.
#[test]
fn inspector_removes_identity_after_a_store_change() {
    use auro::gpui::{Modifiers, Point, px};

    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = edge_order_store();
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.run_until_parked();
    let (x, y, drawer_width) = vcx.read(|cx| {
        let canvas = view.read(cx);
        let point = test_support::edge_click_point(canvas, 1).expect("the cross rope");
        (point.0, point.1, test_support::drawer_width(canvas, cx))
    });
    vcx.simulate_click(Point::new(px(x + drawer_width), px(y)), Modifiers::none());
    vcx.run_until_parked();
    assert!(
        vcx.read(|cx| test_support::connection_popover_open(view.read(cx))),
        "the cross rope opened its inspector"
    );

    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| {
            test_support::remove_local_edge(canvas, ConnectionTarget::Card(1), ConnectionTarget::Card(2), cx)
        })
    });
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| test_support::local_edge_count(view.read(cx))), 0);

    let remove = vcx
        .debug_bounds("connection-remove")
        .expect("the still-open inspector");
    vcx.simulate_click(remove.center(), Modifiers::none());
    vcx.run_until_parked();
    assert!(
        !vcx.read(|cx| test_support::cross_connected(
            view.read(cx),
            1,
            ConnectionTarget::Card(1),
            2,
            ConnectionTarget::Card(4)
        )),
        "the pressed cross edge is removed, not whatever is now at its old index"
    );
    assert_eq!(vcx.read(|cx| test_support::cross_edge_count(view.read(cx))), 0);
}

/// A proxy is a representation only: when it overlaps a real card, the card
/// must keep receiving the press.
#[test]
fn proxy_does_not_steal_clicks_from_an_overlapping_card() {
    use auro::gpui::{AppContext, Modifiers, Point, px};
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = overlapping_proxy_store();
    let canvas_slot = Rc::new(RefCell::new(None));
    let slot = canvas_slot.clone();
    let (_root, vcx) = cx.add_window_view(move |window, cx| {
        let canvas = cx.new(|cx| {
            Canvas::new(
                store.clone(),
                LoadOutcome::Fresh,
                window.window_handle(),
                None,
                cx,
            )
        });
        canvas.update(cx, |canvas, cx| canvas.mount_note_for_test(1, 5, window, cx));
        slot.replace(Some(canvas.clone()));
        gpui_component::Root::new(canvas, window, cx)
    });
    let view = canvas_slot.borrow().clone().expect("canvas entity");
    assert_eq!(
        vcx.read(|cx| test_support::selection_state(view.read(cx)).2),
        Some((1, 5)),
        "the mounted note starts selected"
    );
    let (drawer_width, note) = vcx.read(|cx| {
        let canvas = view.read(cx);
        let bounds = test_support::note_bounds(canvas, 1, cx).expect("mounted note");
        (test_support::drawer_width(canvas, cx), bounds)
    });
    vcx.simulate_click(
        Point::new(
            px(drawer_width + note.x + note.width * 0.5),
            px(note.y + note.height * 0.5),
        ),
        Modifiers::none(),
    );
    vcx.run_until_parked();
    assert_eq!(
        vcx.read(|cx| test_support::selection_state(view.read(cx)).2),
        Some((1, 5)),
        "the real note under the proxy stays clickable: note={note:?}, drawer={drawer_width}"
    );
    assert!(!vcx.read(|cx| test_support::connection_popover_open(view.read(cx))));
    assert_eq!(vcx.read(|cx| test_support::cross_edge_count(view.read(cx))), 1);
}

/// A proxy centred on a large remote card would be painted underneath it and
/// never seen. The default position must sit just outside that card.
#[test]
fn proxy_default_position_sits_outside_the_remote_card() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = cross_workspace_store(true);
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.run_until_parked();

    let proxies = vcx.read(|cx| test_support::proxy_centres(view.read(cx), cx));
    let (_, _, centre) = proxies
        .iter()
        .find(|(workspace, _, _)| *workspace == 2)
        .expect("Auri renders a proxy for the Auro agent");
    let remote = store
        .workspaces()
        .iter()
        .find(|workspace| workspace.id == 2)
        .and_then(|workspace| workspace.canvas.agent(4))
        .expect("remote agent");
    assert!(
        centre.y + crate::theme::PROXY_CARD_HEIGHT * 0.5 <= remote.bounds.y,
        "the proxy must sit above the remote card, not be hidden behind it: {centre:?} vs {:?}",
        remote.bounds
    );
}

#[test]
fn proxy_scales_with_zoom_like_other_cards() {
    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = cross_workspace_store(true);
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.run_until_parked();

    let full = vcx
        .debug_bounds("proxy-card-2-c4")
        .expect("proxy at 100% zoom")
        .size
        .width;
    vcx.update(|_, cx| {
        view.update(cx, |canvas, cx| test_support::set_zoom(canvas, 0.5, cx))
    });
    vcx.run_until_parked();
    let half = vcx
        .debug_bounds("proxy-card-2-c4")
        .expect("proxy at 50% zoom")
        .size
        .width;
    let ratio = f32::from(half) / f32::from(full);
    assert!(
        (ratio - 0.5).abs() < 0.02,
        "the proxy must scale like terminals and agents: full={full:?} half={half:?} ratio={ratio}"
    );
}

/// Cancelling after the cross gesture followed the user into another workspace
/// must clear the source card there, not only the now-active workspace.
#[test]
fn empty_canvas_cancel_clears_a_source_left_in_another_workspace() {
    use auro::gpui::{Modifiers, MouseButton, Point, px};

    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = cross_workspace_store(false);
    let (view, vcx) = cx.add_window_view(|window, cx| {
        let mut canvas = Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        );
        test_support::mount_agent_for_test(&mut canvas, 1, 1, cx);
        canvas.select(1, 1, cx);
        canvas
    });
    vcx.run_until_parked();

    let bolt = vcx.debug_bounds("island-connect").expect("admin bolt");
    vcx.simulate_click(bolt.center(), Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| {
        view.update(cx, |canvas, cx| test_support::activate_via_drawer(canvas, 2, window, cx))
    });
    vcx.run_until_parked();

    vcx.simulate_click(Point::new(px(900.0), px(600.0)), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        vcx.read(|cx| test_support::connect_state(view.read(cx)).0),
        None,
        "an empty-canvas press cancels the gesture"
    );

    vcx.update(|window, cx| {
        view.update(cx, |canvas, cx| test_support::activate_via_drawer(canvas, 1, window, cx))
    });
    vcx.run_until_parked();
    assert_eq!(
        vcx.read(|cx| test_support::card_connect_flags(view.read(cx), 1, 1, cx)),
        Some((false, false)),
        "returning to the source workspace must clear its connection highlight"
    );
    let (drawer_width, before) = vcx.read(|cx| {
        let canvas = view.read(cx);
        (
            test_support::drawer_width(canvas, cx),
            test_support::agent_bounds(canvas, 1, 1).expect("source card"),
        )
    });
    let grab = Point::new(
        px(drawer_width + before.x + before.width * 0.5),
        px(before.y + 16.0),
    );
    let grab_x = f32::from(grab.x);
    let grab_y = f32::from(grab.y);
    vcx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    vcx.simulate_mouse_move(
        Point::new(px(grab_x + 60.0), px(grab_y + 40.0)),
        Some(MouseButton::Left),
        Modifiers::none(),
    );
    vcx.run_until_parked();
    vcx.simulate_mouse_move(
        Point::new(px(grab_x + 120.0), px(grab_y + 80.0)),
        Some(MouseButton::Left),
        Modifiers::none(),
    );
    vcx.run_until_parked();
    vcx.simulate_mouse_up(
        Point::new(px(grab_x + 120.0), px(grab_y + 80.0)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.run_until_parked();
    let after = vcx.read(|cx| test_support::agent_bounds(view.read(cx), 1, 1).unwrap());
    assert!(
        after.x > before.x + 50.0 && after.y > before.y + 30.0,
        "the source card must be movable after the cancel: before={before:?} after={after:?}"
    );
}

/// A card in the workspace reached through the drawer must complete the cross
/// edge, not fall back to a local edge in the newly active workspace.
#[test]
fn clicking_the_activated_workspace_card_creates_the_cross_edge() {
    use auro::gpui::{Modifiers, Point, px};

    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = cross_workspace_store(false);
    let (view, vcx) = cx.add_window_view(|window, cx| {
        let mut canvas = Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        );
        test_support::mount_agent_for_test(&mut canvas, 1, 1, cx);
        test_support::mount_agent_for_test(&mut canvas, 2, 4, cx);
        canvas.select(1, 1, cx);
        canvas
    });
    vcx.run_until_parked();

    let bolt = vcx.debug_bounds("island-connect").expect("admin bolt");
    vcx.simulate_click(bolt.center(), Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| {
        view.update(cx, |canvas, cx| test_support::activate_via_drawer(canvas, 2, window, cx))
    });
    vcx.run_until_parked();
    assert_eq!(
        vcx.read(|cx| test_support::card_connect_flags(view.read(cx), 2, 4, cx)),
        Some((true, false)),
        "the activated workspace card is armed as a connection target"
    );

    let (drawer_width, target) = vcx.read(|cx| {
        let canvas = view.read(cx);
        (
            test_support::drawer_width(canvas, cx),
            test_support::agent_bounds(canvas, 2, 4).expect("target card"),
        )
    });
    vcx.simulate_click(
        Point::new(
            px(drawer_width + target.x + target.width * 0.5),
            px(target.y + 16.0),
        ),
        Modifiers::none(),
    );
    vcx.run_until_parked();

    assert!(
        vcx.read(|cx| test_support::cross_connected(
            view.read(cx),
            1,
            ConnectionTarget::Card(1),
            2,
            ConnectionTarget::Card(4)
        )),
        "the click must create the workspace-qualified edge"
    );
    assert!(!vcx.read(|cx| test_support::cross_connect_open(view.read(cx))));
    assert!(
        vcx.debug_bounds("proxy-card-1-c1").is_some(),
        "the new edge shows the source workspace as a proxy"
    );
}

/// The proxy is a window-like representation: dragging it moves its persisted
/// position without touching the edge or either workspace's real cards.
#[test]
fn proxy_card_moves_when_dragged() {
    use auro::gpui::{Modifiers, MouseButton, Point, px};

    use crate::canvas::Canvas;
    use crate::config::LoadOutcome;

    let mut cx = cross_test_cx();
    let store = cross_workspace_store(true);
    let (view, vcx) = cx.add_window_view(|window, cx| {
        Canvas::new(
            store.clone(),
            LoadOutcome::Fresh,
            window.window_handle(),
            None,
            cx,
        )
    });
    vcx.run_until_parked();

    let before = vcx
        .debug_bounds("proxy-card-2-c4")
        .expect("proxy for the remote endpoint")
        .center();
    let before_x = f32::from(before.x);
    let before_y = f32::from(before.y);
    vcx.simulate_mouse_down(before, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(
        Point::new(px(before_x + 70.0), px(before_y + 40.0)),
        Some(MouseButton::Left),
        Modifiers::none(),
    );
    vcx.run_until_parked();
    vcx.simulate_mouse_move(
        Point::new(px(before_x + 140.0), px(before_y + 90.0)),
        Some(MouseButton::Left),
        Modifiers::none(),
    );
    vcx.run_until_parked();
    vcx.simulate_mouse_up(
        Point::new(px(before_x + 140.0), px(before_y + 90.0)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.run_until_parked();

    let moved = vcx
        .read(|cx| test_support::proxy_position(view.read(cx), 1, "2:c4"))
        .expect("the drag stores the proxy centre on the viewing workspace");
    let drawer_width = vcx.read(|cx| test_support::drawer_width(view.read(cx), cx));
    assert!(
        moved.x > before_x - drawer_width + 100.0 && moved.y > before_y + 60.0,
        "the proxy must follow the drag: stage=({before_x:?} - {drawer_width}, {before_y}) moved={moved:?}"
    );
    assert_eq!(
        vcx.read(|cx| test_support::cross_edge_count(view.read(cx))),
        1,
        "moving the proxy never mutates the edge"
    );
}

