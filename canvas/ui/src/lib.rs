//! Host-side view code for the PTY terminal canvas.

pub mod canvas;
pub mod card;
pub mod config;
pub mod pty;
pub mod workspaces;

use auro::{App, gpui::AppContext};

/// Runs the app with the terminal canvas as the `main` window's root view.
pub fn run() {
    App::new()
        .setup(|cx, _| gpui_component::init(cx))
        .root_with("main", |window, cx| {
            let view = cx.new(|_cx| canvas::Canvas::new());
            cx.new(|cx| gpui_component::Root::new(view, window, cx))
        })
        .run();
}
