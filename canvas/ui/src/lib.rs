//! Host-side view code for the PTY terminal canvas.

pub mod canvas;
pub mod card;
pub mod pty;

use auro::{App, gpui::AppContext};

/// Runs the app with the terminal canvas as the `main` window's root view.
pub fn run() {
    App::new()
        .root_with("main", |_window, cx| cx.new(|_cx| canvas::Canvas::new()))
        .run();
}
