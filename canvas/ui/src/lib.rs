//! Host-side view code for the PTY terminal canvas.

pub mod canvas;
pub mod card;
pub mod config;
pub mod pty;
pub mod theme;
pub mod workspaces;

use auro::{App, assets::EmbeddedAssets, gpui::AppContext};
use canvas_types::WorkspaceStore;

use crate::config::LoadOutcome;

/// Runs the app with the terminal canvas as the `main` window's root view.
pub fn run() {
    let mut store = WorkspaceStore::new();
    let load: LoadOutcome = config::load(&mut store);
    App::new()
        .assets(EmbeddedAssets::new(&[
            ("star.svg", include_bytes!("../../icons/star.svg")),
            ("folder.svg", include_bytes!("../../icons/folder.svg")),
            ("bolt.svg", include_bytes!("../../icons/bolt.svg")),
            ("globe.svg", include_bytes!("../../icons/globe.svg")),
            ("plus.svg", include_bytes!("../../icons/plus.svg")),
            ("search.svg", include_bytes!("../../icons/search.svg")),
            ("x.svg", include_bytes!("../../icons/x.svg")),
            ("terminal.svg", include_bytes!("../../icons/terminal.svg")),
            ("trash.svg", include_bytes!("../../icons/trash.svg")),
        ]))
        .setup(|cx, _| {
            gpui_component::init(cx);
            gpui_component::theme::Theme::change(
                gpui_component::theme::ThemeMode::Dark,
                None,
                cx,
            );
            let theme = gpui_component::theme::Theme::global_mut(cx);
            theme.radius = auro::gpui::px(crate::theme::RADIUS_CONTROL);
            theme.radius_lg = auro::gpui::px(crate::theme::RADIUS_DIALOG);
        })
        .root_with("main", move |window, cx| {
            let view = cx.new(|cx| canvas::Canvas::new(store.clone(), load.clone(), cx));
            view.update(cx, |canvas, cx| canvas.bind_actions(cx));
            cx.new(|cx| gpui_component::Root::new(view, window, cx))
        })
        .run();
}

