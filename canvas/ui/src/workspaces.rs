//! Workspace drawer: filter, creation form and native directory picker.

use auro::gpui::{
    App, Context, Entity, Image, ImageFormat, ImageSource, MouseButton, PathPromptOptions, Render,
    Subscription, Window, div, img, px, rgb,
};
use auro::prelude::*;
use canvas_types::{WorkspaceIcon, WorkspaceId, WorkspaceState, WorkspaceStore};
use gpui_component::input::{Input, InputEvent, InputState};
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::{self, LoadOutcome};

const DRAWER_WIDTH: f32 = 200.0;
const ICON_SIZE: f32 = 16.0;
const PALETTE: [WorkspaceIcon; 5] = [
    WorkspaceIcon::Star,
    WorkspaceIcon::Folder,
    WorkspaceIcon::Bolt,
    WorkspaceIcon::Code,
    WorkspaceIcon::Globe,
];

/// What the drawer asks its owner to do. The owner owns the store and persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawerAction {
    Create {
        name: String,
        icon: WorkspaceIcon,
        root: PathBuf,
    },
    Delete(WorkspaceId),
    Select(WorkspaceId),
}

/// Case-insensitive substring match. Accents are deliberately not normalized.
fn filter_matches(name: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    name.to_lowercase().contains(&query.to_lowercase())
}

/// Sidebar with the workspace list, a filter and an inline creation form.
pub struct WorkspaceDrawer {
    store: WorkspaceStore,
    load: LoadOutcome,
    filter: String,
    filter_input: Option<Entity<InputState>>,
    filter_subscription: Option<Subscription>,
    form_open: bool,
    form_name: String,
    name_input: Option<Entity<InputState>>,
    name_subscription: Option<Subscription>,
    form_root: Option<PathBuf>,
    root_input: Option<Entity<InputState>>,
    root_subscription: Option<Subscription>,
    form_icon: WorkspaceIcon,
    form_error: Option<String>,
    action_handler: Option<Box<dyn Fn(DrawerAction, &mut App)>>,
    restart_handler: Option<Box<dyn Fn(&WorkspaceStore, &mut App)>>,
}

impl WorkspaceDrawer {
    pub fn new(load: LoadOutcome) -> Self {
        let mut store = WorkspaceStore::new();
        let _ = config::load(&mut store);
        Self {
            store,
            load,
            filter: String::new(),
            filter_input: None,
            filter_subscription: None,
            form_open: false,
            form_name: String::new(),
            name_input: None,
            name_subscription: None,
            form_root: None,
            root_input: None,
            root_subscription: None,
            form_icon: WorkspaceIcon::Star,
            form_error: None,
            action_handler: None,
            restart_handler: None,
        }
    }

    /// Consumer of every mutation requested from this drawer.
    pub fn on_action(&mut self, handler: impl Fn(DrawerAction, &mut App) + 'static) {
        self.action_handler = Some(Box::new(handler));
    }

    /// Notified after `config::restart` replaces the store, so the owner can resync.
    pub fn on_restart(&mut self, handler: impl Fn(&WorkspaceStore, &mut App) + 'static) {
        self.restart_handler = Some(Box::new(handler));
    }

    /// Replace the displayed snapshot after the owner mutates its store.
    pub fn set_store(&mut self, store: WorkspaceStore) {
        self.store = store;
    }

    pub fn store(&self) -> &WorkspaceStore {
        &self.store
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn set_filter(&mut self, value: String) {
        self.filter = value;
    }

    pub fn open_form(&mut self) {
        self.form_open = true;
        self.form_error = None;
    }

    pub fn close_form(&mut self) {
        self.form_open = false;
        self.form_error = None;
    }

    pub fn set_form_name(&mut self, value: String) {
        self.form_name = value;
    }

    pub fn set_form_icon(&mut self, icon: WorkspaceIcon) {
        self.form_icon = icon;
    }

    pub fn set_form_root(&mut self, root: Option<PathBuf>) {
        self.form_root = root;
    }

    pub fn set_form_error(&mut self, error: Option<String>) {
        self.form_error = error;
    }

    pub fn is_form_open(&self) -> bool {
        self.form_open
    }

    fn emit(&self, action: DrawerAction, cx: &mut App) {
        if let Some(handler) = &self.action_handler {
            handler(action, cx);
        }
    }

    /// Inputs need a window and context to be created, so they are lazy.
    fn ensure_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_input.is_none() {
            let value = self.filter.clone();
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder("Filtrar")
            });
            self.filter_subscription = Some(cx.subscribe_in(
                &state,
                window,
                |this, state, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.filter = state.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            ));
            self.filter_input = Some(state);
        }
        if self.name_input.is_none() {
            let value = self.form_name.clone();
            let state = cx.new(|cx| InputState::new(window, cx).default_value(value));
            self.name_subscription = Some(cx.subscribe_in(
                &state,
                window,
                |this, state, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.form_name = state.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            ));
            self.name_input = Some(state);
        }
        if self.root_input.is_none() {
            let value = self.root_text();
            let state = cx.new(|cx| InputState::new(window, cx).default_value(value));
            self.root_subscription = Some(cx.subscribe_in(
                &state,
                window,
                |this, state, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = state.read(cx).value().to_string();
                        this.form_root = if value.is_empty() {
                            None
                        } else {
                            Some(PathBuf::from(value))
                        };
                        cx.notify();
                    }
                },
            ));
            self.root_input = Some(state);
        }
    }

    fn root_text(&self) -> String {
        self.form_root
            .as_ref()
            .map(|root| root.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Push programmatic setter values into the inputs; they emit silent changes.
    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_inputs(window, cx);
        for (state, value) in [
            (&self.filter_input, self.filter.clone()),
            (&self.name_input, self.form_name.clone()),
            (&self.root_input, self.root_text()),
        ] {
            if let Some(state) = state {
                if state.read(cx).value().to_string() != value {
                    state.update(cx, |state, cx| state.set_value(value, window, cx));
                }
            }
        }
    }

    fn choose_root(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Escolha a raiz".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(result) => result,
                Err(_) => return,
            };
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| match result {
                    Ok(Some(paths)) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.set_form_root(Some(path));
                            this.form_error = None;
                            cx.notify();
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        this.form_error = Some(format!(
                            "Seletor de pasta indisponível: {error}. Digite a raiz acima."
                        ));
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn submit_form(&mut self, cx: &mut Context<Self>) {
        let name = self.form_name.trim().to_owned();
        if name.is_empty() {
            self.form_error = Some("Informe um nome para o espaço.".into());
            cx.notify();
            return;
        }
        let Some(root) = self.form_root.clone() else {
            self.form_error = Some("Escolha uma raiz para o espaço.".into());
            cx.notify();
            return;
        };
        if !root.is_dir() {
            self.form_error = Some(format!("A raiz {} não é um diretório.", root.display()));
            cx.notify();
            return;
        }
        self.emit(
            DrawerAction::Create {
                name,
                icon: self.form_icon,
                root,
            },
            cx,
        );
        self.close_form();
        cx.notify();
    }


    /// "Recomeçar": back up the damaged config and install a fresh initial store.
    fn restart(&mut self, cx: &mut Context<Self>) {
        let previous = self.store.clone();
        let result = config::restart(&mut self.store).map_err(|error| error.to_string());
        let (load, replaced) = apply_restart_result(&mut self.store, previous, result);
        self.load = load;
        if replaced {
            self.set_store(self.store.clone());
            if let Some(handler) = &self.restart_handler {
                handler(&self.store, cx);
            }
        }
        cx.notify();
    }
}
/// Apply a restart attempt: only a successful write may replace the store in memory.
fn apply_restart_result(
    store: &mut WorkspaceStore,
    previous: WorkspaceStore,
    result: Result<(), String>,
) -> (LoadOutcome, bool) {
    match result {
        Ok(()) => (LoadOutcome::Loaded, true),
        Err(error) => {
            *store = previous;
            (
                LoadOutcome::Corrupt(format!("Recomeçar falhou: {error}")),
                false,
            )
        }
    }
}

fn icon_png(icon: WorkspaceIcon) -> impl IntoElement {
    let bytes: &[u8] = match icon {
        WorkspaceIcon::Star => include_bytes!("../../icons/star.png"),
        WorkspaceIcon::Folder => include_bytes!("../../icons/folder.png"),
        WorkspaceIcon::Bolt => include_bytes!("../../icons/bolt.png"),
        WorkspaceIcon::Code => include_bytes!("../../icons/code.png"),
        WorkspaceIcon::Globe => include_bytes!("../../icons/globe.png"),
    };
    img(ImageSource::Image(Arc::new(Image::from_bytes(
        ImageFormat::Png,
        bytes.to_vec(),
    ))))
    .w(px(ICON_SIZE))
    .h(px(ICON_SIZE))
}

fn monitor(count: usize) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .child(div().w(px(12.0)).h(px(8.0)).bg(rgb(0x8b949e)))
        .child(div().w(px(4.0)).h(px(2.0)).bg(rgb(0x8b949e)))
        .child(format!("{count}"))
}

impl Render for WorkspaceDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_inputs(window, cx);
        let bg = rgb(0x161922);
        let fg = rgb(0xe6edf3);
        let active = self.store.active();
        let filter = self.filter.clone();
        let rows: Vec<WorkspaceState> = self
            .store
            .workspaces()
            .iter()
            .filter(|workspace| filter_matches(&workspace.name, &filter))
            .cloned()
            .collect();
        let banner = match &self.load {
            LoadOutcome::Recovered(reason) => Some(("Configuração recuperada", reason)),
            LoadOutcome::Corrupt(reason) => Some(("Configuração corrompida", reason)),
            LoadOutcome::Fresh | LoadOutcome::Loaded => None,
        };

        div()
            .w(px(DRAWER_WIDTH))
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .bg(bg)
            .text_color(fg)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(
                        self.filter_input
                            .iter()
                            .map(|state| Input::new(state).w(px(140.0))),
                    )
                    .child(
                        Button::new("workspace-add", "+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_form();
                                cx.notify();
                            })),
                    ),
            )
            .when_some(banner, |this, (title, reason)| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_color(rgb(0xe3b341)).child(title.to_owned()))
                        .child(div().text_xs().child(reason.clone()))
                        .child(
                            Button::new("config-restart", "Recomeçar").on_click(
                                cx.listener(|this, _, _, cx| this.restart(cx)),
                            ),
                        ),
                )
            })
            .child(div().text_xs().child("ESPAÇOS DE TRABALHO"))
            .children(rows.into_iter().map(|workspace| {
                let id = workspace.id;
                let is_active = active == Some(id);
                let invalid = workspace.invalid_root;
                div()
                    .id(("workspace-row", id))
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .bg(if is_active {
                        rgb(0x2a3142)
                    } else {
                        rgb(0x161922)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.emit(DrawerAction::Select(id), cx);
                            cx.notify();
                        }),
                    )
                    .child(icon_png(workspace.icon))
                    .child(
                        div()
                            .flex_1()
                            .child(workspace.name)
                            .when(invalid, |this| {
                                this.child(div().text_color(rgb(0xff8888)).child("!"))
                            }),
                    )
                    .child(monitor(workspace.canvas.terminals().len()))
                    .child(
                        div()
                            .id(("workspace-delete-guard", id))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_, _, _, cx| cx.stop_propagation()),
                            )
                            .child(
                                Button::new(format!("workspace-delete-{id}"), "×").on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.emit(DrawerAction::Delete(id), cx);
                                        cx.notify();
                                    }),
                                ),
                            ),
                    )
            }))
            .when(self.form_open, |this| {
                let error = self.form_error.clone();
                let icon = self.form_icon;
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_2()
                        .bg(rgb(0x1a1d26))
                        .child("Novo espaço")
                        .child("Nome")
                        .children(
                            self.name_input
                                .iter()
                                .map(|state| Input::new(state).w_full()),
                        )
                        .child("Ícone")
                        .child(
                            div().flex().gap_1().children(PALETTE.map(|choice| {
                                let selected = choice == icon;
                                div()
                                    .id(("workspace-icon", choice as u64))
                                    .p_1()
                                    .bg(if selected {
                                        rgb(0x2a3142)
                                    } else {
                                        rgb(0x101219)
                                    })
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.set_form_icon(choice);
                                            cx.notify();
                                        }),
                                    )
                                    .child(icon_png(choice))
                            })),
                        )
                        .child("Raiz")
                        .child(
                            Button::new("workspace-pick-root", "Escolher raiz").on_click(
                                cx.listener(|this, _, _, cx| this.choose_root(cx)),
                            ),
                        )
                        .children(
                            self.root_input
                                .iter()
                                .map(|state| Input::new(state).w_full()),
                        )
                        .when_some(error, |this, error| {
                            this.child(div().text_xs().text_color(rgb(0xff8888)).child(error))
                        })
                        .child(
                            div().flex().gap_2().child(
                                Button::new("workspace-create", "Criar").on_click(
                                    cx.listener(|this, _, _, cx| this.submit_form(cx)),
                                ),
                            ).child(
                                Button::new("workspace-cancel", "Cancelar").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.close_form();
                                        cx.notify();
                                    }),
                                ),
                            ),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use canvas_types::{WorkspaceIcon, WorkspaceStore};
    use std::path::PathBuf;

    use super::{LoadOutcome, apply_restart_result, filter_matches};

    #[test]
    fn empty_query_matches_every_name() {
        assert!(filter_matches("AURI", ""));
        assert!(filter_matches("", ""));
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        assert!(filter_matches("Projeto Aurora", "AUR"));
        assert!(filter_matches("AURI", "uri"));
        assert!(filter_matches("AURI", "URI"));
        assert!(!filter_matches("AURI", "xyz"));
    }

    #[test]
    fn filter_does_not_normalize_accents() {
        assert!(filter_matches("São Paulo", "são"));
        assert!(!filter_matches("Sao Paulo", "são"));
    }

    #[test]
    fn failed_restart_preserves_previous_store_and_cards() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add(
            "A".into(),
            WorkspaceIcon::Star,
            PathBuf::from("/var/tmp/auro-e2e/rootA"),
        );
        store.workspace_mut(workspace.id).unwrap().canvas.add();
        let previous = store.clone();

        // config::restart replaces the store in memory before a failing write.
        store = WorkspaceStore::new();
        let (outcome, replaced) =
            apply_restart_result(&mut store, previous.clone(), Err("write failed".into()));

        assert_eq!(
            outcome,
            LoadOutcome::Corrupt("Recomeçar falhou: write failed".into())
        );
        assert!(!replaced);
        assert_eq!(store, previous);
        assert_eq!(store.workspaces()[0].canvas.terminals().len(), 1);
    }
}
