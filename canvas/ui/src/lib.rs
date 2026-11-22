//! Host-side view code for the PTY terminal canvas.

pub mod canvas;
pub mod card;
pub mod config;
pub mod pty;
pub mod workspaces;

use auro::{App, gpui::AppContext};
use canvas_types::WorkspaceStore;

use crate::config::LoadOutcome;

/// Runs the app with the terminal canvas as the `main` window's root view.
pub fn run() {
    let mut store = WorkspaceStore::new();
    let load: LoadOutcome = config::load(&mut store);
    App::new()
        .setup(|cx, _| gpui_component::init(cx))
        .root_with("main", move |window, cx| {
            let view = cx.new(|cx| canvas::Canvas::new(store.clone(), load.clone(), cx));
            view.update(cx, |canvas, cx| canvas.bind_actions(cx));
            cx.new(|cx| gpui_component::Root::new(view, window, cx))
        })
        .run();
}

