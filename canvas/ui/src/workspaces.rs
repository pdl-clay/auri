//! Workspace drawer: filter, creation dialog and native directory picker.

use auro::gpui::{
    App, Context, CursorStyle, DragMoveEvent, Entity, MouseButton, MouseDownEvent, PathPromptOptions,
    Pixels, Point, Render, Subscription, WeakEntity, Window, div, px, rgb, rgba, svg,
};
use auro::prelude::*;
use canvas_types::{WorkspaceColor, WorkspaceIcon, WorkspaceId, WorkspaceState, WorkspaceStore};
use gpui_component::{
    WindowExt,
    button::{Button, ButtonVariant, ButtonVariants as _},
    dialog::{Dialog, DialogButtonProps},
    input::{Input, InputEvent, InputState},
};
use std::path::PathBuf;

use crate::config::{self, LoadOutcome};
use crate::theme::*;

const PALETTE: [WorkspaceIcon; 5] = [
    WorkspaceIcon::Star,
    WorkspaceIcon::Folder,
    WorkspaceIcon::Bolt,
    WorkspaceIcon::Code,
    WorkspaceIcon::Globe,
];

const COLOR_SWATCHES: [WorkspaceColor; 4] = [
    WorkspaceColor::Brass,
    WorkspaceColor::Steel,
    WorkspaceColor::Red,
    WorkspaceColor::Green,
];

/// What the drawer asks its owner to do. The owner owns the store and persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawerAction {
    Create {
        name: String,
        icon: WorkspaceIcon,
        color: WorkspaceColor,
        root: PathBuf,
    },
    Update {
        id: WorkspaceId,
        name: String,
        icon: WorkspaceIcon,
        color: WorkspaceColor,
        root: PathBuf,
    },
    Unload(WorkspaceId),
    /// Removes the workspace; used by both the row X and "Fechar" in the menu.
    Delete(WorkspaceId),
    Select(WorkspaceId),
}

/// Right-click target: the empty drawer or one of its workspace rows.
#[derive(Clone, Copy)]
enum MenuTarget {
    New,
    Workspace(WorkspaceId),
}

/// Case-insensitive substring match. Accents are deliberately not normalized.
fn filter_matches(name: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    name.to_lowercase().contains(&query.to_lowercase())
}

/// Rows show every saved card in a workspace, loaded or not.
fn workspace_item_count(workspace: &WorkspaceState) -> usize {
    workspace.canvas.terminals().len() + workspace.canvas.agents().len()
}

#[derive(Clone, Copy)]
struct DrawerDragToken;

impl Render for DrawerDragToken {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Sidebar with the workspace list, a filter and a centered creation dialog.
pub struct WorkspaceDrawer {
    store: WorkspaceStore,
    load: LoadOutcome,
    filter: String,
    filter_input: Option<Entity<InputState>>,
    filter_subscription: Option<Subscription>,
    form_open: bool,
    /// `Some` while the shared form edits an existing workspace.
    form_id: Option<WorkspaceId>,
    form_name: String,
    name_input: Option<Entity<InputState>>,
    name_subscription: Option<Subscription>,
    form_root: Option<PathBuf>,
    root_input: Option<Entity<InputState>>,
    root_subscription: Option<Subscription>,
    form_icon: WorkspaceIcon,
    form_error: Option<String>,
    action_handler: Option<Box<dyn Fn(DrawerAction, &mut App)>>,
    form_color: WorkspaceColor,
    restart_handler: Option<Box<dyn Fn(&WorkspaceStore, &mut App)>>,
    context_menu: Option<(MenuTarget, Point<Pixels>)>,
    drawer_width: f32,
    drag_start: Option<f32>,
    /// Verified release available for the next restart; shown in the footer.
    update_available: Option<String>,
}

impl WorkspaceDrawer {
    pub fn new(load: LoadOutcome) -> Self {
        let store = WorkspaceStore::new();
        Self {
            store,
            load,
            filter: String::new(),
            filter_input: None,
            filter_subscription: None,
            form_open: false,
            form_id: None,
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
            context_menu: None,
            form_color: WorkspaceColor::Brass,
            drawer_width: DRAWER_WIDTH,
            drag_start: None,
            update_available: None,
        }
    }

    pub fn drawer_width(&self) -> f32 {
        self.drawer_width
    }

    pub fn set_update_available(&mut self, version: String, cx: &mut Context<Self>) {
        self.update_available = Some(version);
        cx.notify();
    }

    fn begin_drawer_drag(&mut self, x: f32) {
        self.drag_start = Some(x - self.drawer_width);
    }

    fn apply_drawer_drag(
        &mut self,
        event: &DragMoveEvent<DrawerDragToken>,
        cx: &mut Context<Self>,
    ) {
        let Some(offset) = self.drag_start else {
            return;
        };
        self.drawer_width = (f32::from(event.event.position.x) - offset)
            .clamp(MIN_DRAWER_WIDTH, MAX_DRAWER_WIDTH);
        cx.notify();
    }
    /// Open the shared centered workspace creation dialog.
    pub fn open_create_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_form();
        self.open_form();
        self.show_form_dialog(window, cx);
    }

    /// Open the same form pre-filled with an existing workspace's values.
    pub fn open_edit_dialog(
        &mut self,
        id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.store.workspaces().iter().find(|ws| ws.id == id) else {
            return;
        };
        let (name, icon, color, root) = (
            workspace.name.clone(),
            workspace.icon,
            workspace.color,
            workspace.root.clone(),
        );
        self.reset_form();
        self.form_id = Some(id);
        self.form_name = name;
        self.form_icon = icon;
        self.form_color = color;
        self.form_root = Some(root);
        self.open_form();
        self.show_form_dialog(window, cx);
    }

    fn show_form_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu = None;
        let drawer = cx.weak_entity();
        window.open_dialog(cx, move |dialog, window, cx| {
            workspace_dialog(dialog, window, cx, &drawer)
        });
        cx.notify();
    }

    fn reset_form(&mut self) {
        self.form_id = None;
        self.form_name.clear();
        self.form_root = None;
        self.form_icon = WorkspaceIcon::Star;
        self.form_color = WorkspaceColor::Brass;
        self.form_error = None;
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

    /// Closing clears every form field; `sync_inputs` keeps the visible inputs in step.
    pub fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form_open = false;
        self.reset_form();
        self.sync_inputs(window, cx);
    }

    pub fn set_form_name(&mut self, value: String) {
        self.form_name = value;
    }

    pub fn set_form_icon(&mut self, icon: WorkspaceIcon) {
        self.form_icon = icon;
    }

    pub fn set_form_color(&mut self, color: WorkspaceColor) {
        self.form_color = color;
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
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder("Ex.: Meu projeto")
            });
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
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder("Sem pasta selecionada")
            });
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

    /// The pending form mutation; edit mode reuses the create form verbatim.
    fn form_action(&self, name: String, root: PathBuf) -> DrawerAction {
        match self.form_id {
            Some(id) => DrawerAction::Update {
                id,
                name,
                icon: self.form_icon,
                color: self.form_color,
                root,
            },
            None => DrawerAction::Create {
                name,
                icon: self.form_icon,
                color: self.form_color,
                root,
            },
        }
    }

    fn submit_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let action = self.form_action(name, root);
        self.emit(action, cx);
        self.close_form(window, cx);
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

fn asset_icon_sized(path: &'static str, color: u32, size: f32) -> impl IntoElement {
    svg()
        .path(path)
        .w(px(size))
        .h(px(size))
        .text_color(rgb(color))
}

fn asset_icon(path: &'static str, color: u32) -> impl IntoElement {
    asset_icon_sized(path, color, ICON_SIZE)
}

fn icon_path(icon: WorkspaceIcon) -> &'static str {
    match icon {
        WorkspaceIcon::Star => "star.svg",
        WorkspaceIcon::Folder => "folder.svg",
        WorkspaceIcon::Bolt => "bolt.svg",
        WorkspaceIcon::Code => "code.svg",
        WorkspaceIcon::Globe => "globe.svg",
    }
}

fn icon_svg(icon: WorkspaceIcon, color: u32) -> impl IntoElement {
    asset_icon(icon_path(icon), color)
}

fn row_icon(icon: WorkspaceIcon, color: u32) -> impl IntoElement {
    asset_icon_sized(icon_path(icon), color, ROW_ICON_SIZE)
}

fn delete_icon() -> impl IntoElement {
    svg()
        .path("x.svg")
        .w(px(ROW_ICON_SIZE))
        .h(px(ROW_ICON_SIZE))
        .text_color(rgb(COLOR_MUTED))
        .hover(|this| this.text_color(rgb(COLOR_DANGER)))
}

fn field_label(text: &'static str) -> impl IntoElement {
    div().text_sm().text_color(rgb(COLOR_MUTED)).child(text)
}

/// Root calls this builder on every frame; read the drawer snapshot instead of
/// caching form state here. The input states are reused, never recreated.
fn workspace_dialog(
    dialog: Dialog,
    window: &mut Window,
    cx: &mut App,
    drawer: &WeakEntity<WorkspaceDrawer>,
) -> Dialog {
    let Some(entity) = drawer.upgrade() else {
        return dialog;
    };
    let (_form_name, icon, color, _form_root, error, name_input, root_input) = {
        let state = entity.read(cx);
        (
            state.form_name.clone(),
            state.form_icon,
            state.form_color,
            state.root_text(),
            state.form_error.clone(),
            state.name_input.clone(),
            state.root_input.clone(),
        )
    };
    let editing = entity.read(cx).form_id.is_some();
    let ok_drawer = drawer.clone();
    let cancel_drawer = drawer.clone();
    let pick_drawer = drawer.clone();

    dialog
        .title(
            div()
                .text_lg()
                .text_color(rgb(COLOR_FOREGROUND))
                .child(if editing { "Editar espaço de trabalho" } else { "Novo espaço de trabalho" }),
        )
        .w(px(440.))
        .margin_top((window.viewport_size().height - px(320.0)) / 2.0)
        .overlay(true)
        .rounded(px(RADIUS_DIALOG))
        .border_1()
        .border_color(rgb(COLOR_BORDER))
        .bg(rgb(COLOR_PANEL))
        .text_color(rgb(COLOR_FOREGROUND))
        .button_props(
            DialogButtonProps::default()
                .ok_text(if editing { "Salvar alterações" } else { "Criar espaço" })
                .cancel_text("Cancelar")
                .ok_variant(ButtonVariant::Primary)
                .cancel_variant(ButtonVariant::Ghost),
        )
        .footer(|ok, cancel, window, cx| vec![cancel(window, cx), ok(window, cx)])
        .on_ok(move |_, window, cx| {
            let mut created = false;
            if let Some(entity) = ok_drawer.upgrade() {
                entity.update(cx, |this, cx| {
                    this.submit_form(window, cx);
                    created = this.form_error.is_none();
                    cx.notify();
                });
            }
            created
        })
        .on_cancel(move |_, window, cx| {
            if let Some(entity) = cancel_drawer.upgrade() {
                entity.update(cx, |this, cx| {
                    this.close_form(window, cx);
                    cx.notify();
                });
            }
            true
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(field_label("Nome"))
                .children(name_input.iter().map(|state| {
                    Input::new(state)
                        .w_full()
                        .rounded(px(RADIUS_CONTROL))
                        .bg(rgb(COLOR_ELEVATED))
                        .border_color(rgb(COLOR_BORDER))
                }))
                .child(field_label("Ícone"))
                .child(
                    div().flex().gap_2().children(PALETTE.map(|choice| {
                        let selected = choice == icon;
                        let tile_drawer = drawer.clone();
                        Button::new(("new-workspace-icon", choice as u64))
                            .child(icon_svg(
                                choice,
                                if selected {
                                    COLOR_ACCENT
                                } else {
                                    COLOR_MUTED
                                },
                            ))
                            .w(px(40.))
                            .h(px(40.))
                            .p_0()
                            .rounded(px(RADIUS_CONTROL))
                            .border_1()
                            .border_color(rgb(if selected {
                                COLOR_ACCENT
                            } else {
                                COLOR_BORDER
                            }))
                            .bg(rgb(if selected {
                                COLOR_ELEVATED
                            } else {
                                COLOR_BASE
                            }))
                            .with_variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                if let Some(entity) = tile_drawer.upgrade() {
                                    entity.update(cx, |this, cx| {
                                        this.set_form_icon(choice);
                                        cx.notify();
                                    });
                                }
                            })
                    })),
                )
                .child(field_label("Cor"))
                .child(
                    div().flex().gap_2().children(COLOR_SWATCHES.map(|choice| {
                        let swatch_drawer = drawer.clone();
                        let selected = choice == color;
                        Button::new(("new-workspace-color", choice as u64))
                            .w(px(COLOR_SWATCH_SIZE))
                            .h(px(COLOR_SWATCH_SIZE))
                            .p_0()
                            .rounded(px(RADIUS_CHIP))
                            .border_1()
                            .border_color(rgb(if selected {
                                COLOR_FOREGROUND
                            } else {
                                COLOR_BORDER
                            }))
                            .bg(rgb(workspace_color(choice)))
                            .with_variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                if let Some(entity) = swatch_drawer.upgrade() {
                                    entity.update(cx, |this, cx| {
                                        this.set_form_color(choice);
                                        cx.notify();
                                    });
                                }
                            })
                    })),
                )
                .child(field_label("Raiz"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("new-workspace-pick-root")
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(asset_icon("folder.svg", COLOR_FOREGROUND))
                                        .child("Escolher pasta"),
                                )
                                .px(px(SPACE_PANEL))
                                .py(px(SPACE_NORMAL))
                                .rounded(px(RADIUS_PILL))
                                .bg(rgb(COLOR_ELEVATED))
                                .border_1()
                                .border_color(rgb(COLOR_BORDER))
                                .with_variant(ButtonVariant::Ghost)
                                .on_click({
                                    let pick_drawer = pick_drawer.clone();
                                    move |_, _, cx| {
                                        if let Some(entity) = pick_drawer.upgrade() {
                                            entity.update(cx, |this, cx| this.choose_root(cx));
                                        }
                                    }
                                }),
                        )
                        .children(root_input.iter().map(|state| {
                            Input::new(state)
                                .flex_1()
                                .rounded(px(RADIUS_CONTROL))
                                .bg(rgb(COLOR_ELEVATED))
                                .border_color(rgb(COLOR_BORDER))
                        })),
                )
                .when_some(error, |this, error| {
                    this.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_xs()
                            .text_color(rgb(COLOR_DANGER))
                            .child(error)
                            .child("Corrija o campo indicado e tente novamente."),
                    )
                }),
        )
}

/// Keep the context menu inside the drawer and above the window bottom.
fn menu_origin(
    position: Point<Pixels>,
    viewport_height: f32,
    drawer_width: f32,
    item_count: usize,
) -> (f32, f32) {
    let menu_height =
        CONTEXT_MENU_ITEM_HEIGHT * item_count as f32 + 2.0 * SPACE_TIGHT;
    let max_x = (drawer_width - CONTEXT_MENU_WIDTH - SPACE_TIGHT).max(SPACE_TIGHT);
    let max_y = (viewport_height - menu_height - SPACE_WIDE).max(SPACE_TIGHT);
    (
        f32::from(position.x).clamp(SPACE_TIGHT, max_x),
        f32::from(position.y).clamp(SPACE_TIGHT, max_y),
    )
}

impl Render for WorkspaceDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_inputs(window, cx);
        let active = self.store.active();
        let filter = self.filter.clone();
        let context_menu = self.context_menu.map(|(target, position)| {
            let item_count = match target {
                MenuTarget::New => 1,
                MenuTarget::Workspace(_) => 3,
            };
            (
                target,
                menu_origin(
                    position,
                    f32::from(window.viewport_size().height),
                    self.drawer_width,
                    item_count,
                ),
            )
        });
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
            .w(px(self.drawer_width))
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .bg(rgb(COLOR_PANEL))
            .border_r_1()
            .border_color(rgb(COLOR_BORDER))
            .text_color(rgb(COLOR_FOREGROUND))
            .relative()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.prevent_default();
                    this.context_menu = Some((MenuTarget::New, event.position));
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(self.filter_input.iter().map(|state| {
                        Input::new(state)
                            .flex_1()
                            .rounded(px(RADIUS_CONTROL))
                            .bg(rgb(COLOR_ELEVATED))
                            .border_color(rgb(COLOR_BORDER))
                            .prefix(asset_icon("search.svg", COLOR_MUTED))
                    }))
                    .child(
                        Button::new("workspace-add")
                            .tooltip("Novo espaço")
                            .child(asset_icon("plus.svg", COLOR_MUTED))
                            .w(px(28.))
                            .h(px(28.))
                            .p_0()
                            .rounded(px(RADIUS_CONTROL))
                            .with_variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_create_dialog(window, cx);
                            }))
                    ),
            )
            .when_some(banner, |this, (title, reason)| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .rounded(px(RADIUS_CONTROL))
                        .border_1()
                        .border_color(rgb(COLOR_BORDER))
                        .bg(rgb(COLOR_ELEVATED))
                        .p_2()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(COLOR_ACCENT))
                                .child(title.to_owned()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(COLOR_MUTED))
                                .child(reason.clone()),
                        )
                        .child(
                            Button::new("config-restart")
                                .label("Recomeçar")
                                .rounded(px(RADIUS_CONTROL))
                                .with_variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|this, _, _, cx| this.restart(cx))),
                        ),
                )
            })
            .child(
                div()
                    .mt_4()
                    .mb_1()
                    .text_xs()
                    .text_color(rgb(COLOR_MUTED))
                    .child("Espaços de trabalho"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(WORKSPACE_ROW_GAP))
                    .children(rows.into_iter().map(|workspace| {
                let id = workspace.id;
                let is_active = active == Some(id);
                let invalid = workspace.invalid_root;
                let unloaded = workspace.unloaded;
                let item_count = workspace_item_count(&workspace);
                let terminal_color = if unloaded {
                    COLOR_UNLOADED_ICON
                } else {
                    COLOR_MUTED
                };
                let color = workspace_color(workspace.color);
                div()
                    .id(("workspace-row", id))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py(px(WORKSPACE_ROW_PAD_Y))
                    .rounded(px(RADIUS_CHIP))
                    .bg(if is_active {
                        rgb(COLOR_ELEVATED)
                    } else {
                        rgba(0x00000000)
                    })
                    .border_1()
                    .border_color(if is_active {
                        rgba(COLOR_ROW_BORDER_ACTIVE)
                    } else {
                        rgba(0x00000000)
                    })
                    .when(!is_active, |this| {
                        this.hover(|style| style.bg(rgb(COLOR_ROW_HOVER)))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.emit(DrawerAction::Select(id), cx);
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.context_menu = Some((MenuTarget::Workspace(id), event.position));
                            cx.notify();
                        }),
                    )
                    .child(row_icon(workspace.icon, color))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .child(workspace.name)
                            .when(invalid, |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(COLOR_DANGER))
                                        .child("!"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .relative()
                            .w(px(ROW_ICON_SIZE + COUNT_BADGE_RESERVE))
                            .flex_none()
                            .child(asset_icon_sized("terminal.svg", terminal_color, ROW_ICON_SIZE))
                            .child(
                                div()
                                    .absolute()
                                    .top(px(COUNT_BADGE_OFFSET))
                                    .right_0()
                                    .text_xs()
                                    .text_color(rgb(terminal_color))
                                    .child(format!("{item_count}")),
                            ),
                    )
                    .child(
                        div()
                            .id(("workspace-delete-guard", id))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_, _, _, cx| cx.stop_propagation()),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "workspace-delete-{id}"
                                )))
                                .tooltip("Remover espaço")
                                .child(delete_icon())
                                .w(px(ROW_ICON_SIZE))
                                .h(px(ROW_ICON_SIZE))
                                .p_0()
                                .rounded(px(RADIUS_CONTROL))
                                .with_variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.emit(DrawerAction::Delete(id), cx);
                                    cx.notify();
                                })),
                            ),
                    )
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("drawer-version")
                    .debug_selector(|| "drawer-version".to_string())
                    .flex()
                    .flex_col()
                    .gap(px(SPACE_TIGHT))
                    .border_t_1()
                    .border_color(rgb(COLOR_BORDER))
                    .pt(px(SPACE_TIGHT))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(COLOR_MUTED))
                            .child(format!("Auri v{}", crate::APP_VERSION)),
                    )
                    .when_some(self.update_available.clone(), |this, version| {
                        this.child(
                            div()
                                .id("drawer-update-status")
                                .debug_selector(|| "drawer-update-status".to_string())
                                .text_xs()
                                .text_color(rgb(COLOR_ACCENT))
                                .child(format!("v{version} disponível")),
                        )
                    }),
            )
            .when_some(context_menu, |this, (target, (x, y))| {
                let this = this.child(
                    div()
                        .absolute()
                        .left(px(0.0))
                        .top(px(0.0))
                        .size_full()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.context_menu = None;
                                cx.notify();
                            }),
                        ),
                );
                let menu = div()
                    .absolute()
                    .top(px(y))
                    .left(px(x))
                    .w(px(CONTEXT_MENU_WIDTH))
                    .flex()
                    .flex_col()
                    .p(px(SPACE_TIGHT))
                    .rounded(px(RADIUS_CONTROL))
                    .bg(rgb(COLOR_ELEVATED))
                    .border_1()
                    .border_color(rgb(COLOR_BORDER));
                match target {
                    MenuTarget::New => this.child(menu.child(
                        div()
                            .id("drawer-new-workspace")
                            .h(px(CONTEXT_MENU_ITEM_HEIGHT))
                            .flex()
                            .items_center()
                            .gap(px(SPACE_NORMAL))
                            .px(px(SPACE_NORMAL))
                            .rounded(px(RADIUS_CHIP))
                            .hover(|style| style.bg(rgb(COLOR_BORDER)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.context_menu = None;
                                    this.open_create_dialog(window, cx);
                                }),
                            )
                            .child(asset_icon("plus.svg", COLOR_FOREGROUND))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(COLOR_FOREGROUND))
                                    .child("Novo espaço de trabalho"),
                            ),
                    )),
                    MenuTarget::Workspace(id) => this.child(
                        menu.child(
                            div()
                                .id(("drawer-workspace-unload", id))
                                .h(px(CONTEXT_MENU_ITEM_HEIGHT))
                                .flex()
                                .items_center()
                                .gap(px(SPACE_NORMAL))
                                .px(px(SPACE_NORMAL))
                                .rounded(px(RADIUS_CHIP))
                                .hover(|style| style.bg(rgb(COLOR_BORDER)))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.context_menu = None;
                                        this.emit(DrawerAction::Unload(id), cx);
                                        cx.notify();
                                    }),
                                )
                                .child(asset_icon("unload.svg", COLOR_FOREGROUND))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(COLOR_FOREGROUND))
                                        .child("Descarregar"),
                                ),
                        )
                        .child(
                            div()
                                .id(("drawer-workspace-edit", id))
                                .h(px(CONTEXT_MENU_ITEM_HEIGHT))
                                .flex()
                                .items_center()
                                .gap(px(SPACE_NORMAL))
                                .px(px(SPACE_NORMAL))
                                .rounded(px(RADIUS_CHIP))
                                .hover(|style| style.bg(rgb(COLOR_BORDER)))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, window, cx| {
                                        this.context_menu = None;
                                        this.open_edit_dialog(id, window, cx);
                                    }),
                                )
                                .child(asset_icon("edit.svg", COLOR_FOREGROUND))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(COLOR_FOREGROUND))
                                        .child("Editar"),
                                ),
                        )
                        .child(
                            div()
                                .id(("drawer-workspace-close", id))
                                .h(px(CONTEXT_MENU_ITEM_HEIGHT))
                                .flex()
                                .items_center()
                                .gap(px(SPACE_NORMAL))
                                .px(px(SPACE_NORMAL))
                                .rounded(px(RADIUS_CHIP))
                                .hover(|style| style.bg(rgb(COLOR_BORDER)))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.context_menu = None;
                                        this.emit(DrawerAction::Delete(id), cx);
                                        cx.notify();
                                    }),
                                )
                                .child(asset_icon("x.svg", COLOR_DANGER))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(COLOR_DANGER))
                                        .child("Fechar"),
                                ),
                        ),
                    ),
                }
            })
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .w(px(DRAWER_HANDLE_WIDTH))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .id("drawer-resize-handle")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.begin_drawer_drag(f32::from(event.position.x));
                        }),
                    )
                    .on_drag(DrawerDragToken, |_, _, _, cx| cx.new(|_| DrawerDragToken))
                    .on_drag_move(cx.listener(
                        |this, event: &DragMoveEvent<DrawerDragToken>, _, cx| {
                            this.apply_drawer_drag(event, cx);
                        },
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use canvas_types::{AgentKind, CardBounds, WorkspaceColor, WorkspaceIcon, WorkspaceStore};
    use std::path::PathBuf;

    use super::{
        DrawerAction, LoadOutcome, WorkspaceDrawer, apply_restart_result, filter_matches, menu_origin,
        workspace_item_count,
    };
    use auro::gpui::{Point, px};

    #[test]
    fn empty_query_matches_every_name() {
        assert!(filter_matches("AURI", ""));
        assert!(filter_matches("", ""));
    }

    #[test]
    fn drawer_menu_follows_click_and_stays_inside_drawer() {
        assert_eq!(
            menu_origin(Point::new(px(30.0), px(140.0)), 900.0, 224.0, 1),
            (30.0, 140.0)
        );
        assert_eq!(
            menu_origin(Point::new(px(900.0), px(890.0)), 900.0, 224.0, 1),
            (224.0 - 180.0 - 4.0, 900.0 - 32.0 - 12.0 - 8.0)
        );
        assert_eq!(
            menu_origin(Point::new(px(900.0), px(890.0)), 900.0, 224.0, 3),
            (224.0 - 180.0 - 4.0, 900.0 - 3.0 * 32.0 - 12.0 - 8.0)
        );
    }

    #[test]
    fn workspace_row_counts_terminals_and_agents() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/tmp/auro-count"));
        let id = workspace.id;
        assert_eq!(workspace_item_count(&workspace), 0);

        let canvas = &mut store.workspace_mut(id).unwrap().canvas;
        canvas.add();
        let bounds = CardBounds { x: 0.0, y: 0.0, width: 10.0, height: 10.0 };
        canvas.add_agent(AgentKind::Claude, bounds);
        canvas.add_agent(AgentKind::Codex, bounds);
        store.workspace_mut(id).unwrap().mark_loaded();
        let loaded = store.workspaces().iter().find(|item| item.id == id).unwrap();
        assert_eq!(workspace_item_count(loaded), 3);

        let mut unloaded = loaded.clone();
        unloaded.unloaded = true;
        assert_eq!(workspace_item_count(&unloaded), 3);
    }

    #[test]
    fn edit_form_emits_update_with_all_edited_fields() {
        let mut drawer = WorkspaceDrawer::new(LoadOutcome::Loaded);
        drawer.form_id = Some(7);
        drawer.set_form_icon(WorkspaceIcon::Globe);
        drawer.set_form_color(WorkspaceColor::Green);

        assert_eq!(
            drawer.form_action("Novo nome".into(), PathBuf::from("/novo")),
            DrawerAction::Update {
                id: 7,
                name: "Novo nome".into(),
                icon: WorkspaceIcon::Globe,
                color: WorkspaceColor::Green,
                root: PathBuf::from("/novo"),
            }
        );

        drawer.form_id = None;
        assert!(matches!(
            drawer.form_action("Outro".into(), PathBuf::from("/outro")),
            DrawerAction::Create { .. }
        ));
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
