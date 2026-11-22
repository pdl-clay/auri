//! Canvas view: grid background, toolbar and the lifecycle of PTY-backed cards.

use auro::gpui::{
    Bounds, Context, Entity, MouseButton, MouseDownEvent, Point, Render, Size, Window, canvas,
    fill, px, rgb,
};
use auro::prelude::*;
use canvas_types::{CanvasState, TerminalId};

use crate::card::{TerminalCard, terminal_grid};
use crate::pty::PtySession;

const GRID_STEP: f32 = 24.0;
const CARD_WIDTH: f32 = 480.0;
const CARD_HEIGHT: f32 = 320.0;

/// Owns the logical canvas state, its terminal cards and the last spawn error.
pub struct Canvas {
    pub(crate) state: CanvasState,
    cards: Vec<(TerminalId, Entity<TerminalCard>)>,
    error: Option<String>,
}

impl Canvas {
    pub fn new() -> Self {
        Self {
            state: CanvasState::new(),
            cards: Vec::new(),
            error: None,
        }
    }

    #[cfg(test)]
    fn new_for_test() -> Self {
        Self::new()
    }

    pub fn add_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.state.add().id;
        let offset = GRID_STEP * (self.cards.len() % 8) as f32;
        let bounds = Bounds::new(
            Point::new(px(16.0 + offset), px(16.0 + offset)),
            Size::new(px(CARD_WIDTH), px(CARD_HEIGHT)),
        );
        let (cols, rows) = terminal_grid(&bounds);
        let pty = match PtySession::spawn_shell(cols, rows) {
            Ok(pty) => pty,
            Err(error) => {
                // A failed fork must not leave a phantom entry in the state.
                self.state.remove(id);
                self.error = Some(format!("Terminal {id} não iniciou: {error}"));
                cx.notify();
                return;
            }
        };
        self.error = None;
        let canvas = cx.entity().downgrade();
        let card = cx.new(|cx| {
            TerminalCard::new(id, bounds, canvas.clone(), pty, (cols, rows), cx)
        });
        card.read(cx).terminal.read(cx).focus_handle().focus(window);
        self.cards.push((id, card));
        cx.notify();
    }

    pub fn remove(&mut self, id: TerminalId) {
        if self.state.remove(id).is_some() {
            // Dropping the last strong handle drops TerminalCard, and PtySession::Drop
            // terminates the shell's whole process group.
            self.cards.retain(|(card_id, _)| *card_id != id);
        }
    }

    pub fn remove_selected(&mut self) {
        if let Some(id) = self.state.selected() {
            self.remove(id);
        }
    }

    pub fn select(&mut self, id: TerminalId, cx: &mut Context<Self>) {
        if self.state.selected() != Some(id) {
            self.state.select(id);
            cx.notify();
        }
    }

    pub fn mark_exited(&mut self, id: TerminalId, cx: &mut Context<Self>) {
        self.state.mark_exited(id);
        if let Some((_, card)) = self.cards.iter().find(|(card_id, _)| *card_id == id) {
            card.update(cx, |card, cx| card.set_exited(cx));
        }
        cx.notify();
    }
}

impl Render for Canvas {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.state.selected();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0f1117))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .bg(rgb(0x1a1d26))
                    .child(Button::new("add-terminal", "Adicionar terminal").on_click(
                        cx.listener(|this, _, window, cx| this.add_terminal(window, cx)),
                    ))
                    .child(Button::new("remove-selected", "Apagar selecionado").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.remove_selected();
                            cx.notify();
                        }),
                    ))
                    .when_some(selected, |bar, id| {
                        bar.child(format!("Selecionado: Terminal {id}"))
                    })
                    .children(self.error.iter().map(|message| {
                        div().text_color(rgb(0xff8888)).child(message.clone())
                    })),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        canvas(
                            |_, _, _| {},
                            |bounds, _, window, _| {
                                let origin = bounds.origin;
                                let width = f32::from(bounds.size.width);
                                let height = f32::from(bounds.size.height);
                                let mut x = GRID_STEP;
                                while x < width {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            Point::new(px(f32::from(origin.x) + x), origin.y),
                                            Size::new(px(1.0), bounds.size.height),
                                        ),
                                        rgb(0x242936),
                                    ));
                                    x += GRID_STEP;
                                }
                                let mut y = GRID_STEP;
                                while y < height {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            Point::new(origin.x, px(f32::from(origin.y) + y)),
                                            Size::new(bounds.size.width, px(1.0)),
                                        ),
                                        rgb(0x242936),
                                    ));
                                    y += GRID_STEP;
                                }
                            },
                        )
                        .absolute()
                        .left(px(0.0))
                        .top(px(0.0))
                        .size_full(),
                    )
                    .children(self.cards.iter().map(|(id, card)| {
                        let id = *id;
                        div()
                            .id(("card-wrapper", id))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                    this.select(id, cx);
                                }),
                            )
                            .child(card.clone())
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_selected_drops_state_entry() {
        let mut canvas = Canvas::new_for_test();
        canvas.state.add();
        canvas.remove_selected();
        assert!(canvas.state.terminals().is_empty());
    }
}
