//! GPUI presentation for the agent chat view.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use auro::gpui::{
    AnyElement, App, ClipboardItem, Context, Entity, IntoElement, ParentElement, Render,
    ScrollHandle, SharedString, Styled, Window, div, px, relative, rgb,
};
use auro::prelude::*;
use gpui_component::{
    button::{Button, ButtonVariant, ButtonVariants as _},
    input::{Input, InputState},
    text::TextView,
};

use crate::chat::{ChatFormat, ChatMessage, ChatRole, ChatStatus};
use crate::theme::*;

pub struct ChatView {
    pub input: Entity<InputState>,
    messages: Vec<ChatMessage>,
    status: Option<ChatStatus>,
    scroll: ScrollHandle,
}

impl ChatView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Mensagem para o agente")
                .auto_grow(CHAT_INPUT_MIN_ROWS, CHAT_INPUT_MAX_ROWS)
        });
        Self {
            input,
            messages: Vec::new(),
            status: None,
            scroll: ScrollHandle::new(),
        }
    }

    pub fn set_messages(&mut self, messages: Vec<ChatMessage>, cx: &mut Context<Self>) {
        self.messages = messages;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn push_message(&mut self, message: ChatMessage, cx: &mut Context<Self>) {
        self.messages.push(message);
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn set_info(&mut self, text: String, cx: &mut Context<Self>) {
        self.set_messages(
            vec![ChatMessage {
                role: ChatRole::Info,
                text,
                format: ChatFormat::Plain,
            }],
            cx,
        );
    }

    pub fn set_status(&mut self, status: Option<ChatStatus>, cx: &mut Context<Self>) {
        self.status = status;
        cx.notify();
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn value(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn render_message(
        &self,
        index: usize,
        message: &ChatMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = format!("chat-message-{index}");
        let content: AnyElement = match message.format {
            ChatFormat::Markdown => TextView::markdown(
                SharedString::from(format!("chat-md-{index}")),
                message.text.clone(),
                window,
                cx,
            )
            .selectable(true)
            .text_size(px(CHAT_TEXT_SIZE))
            .text_color(rgb(COLOR_FOREGROUND))
            .code_block_actions(|code, _, _cx| {
                let text = code.code();
                Button::new(code_button_id(&text))
                    .label("Copiar")
                    .with_variant(ButtonVariant::Ghost)
                    .h(px(CHAT_COPY_HEIGHT))
                    .rounded(px(RADIUS_CHIP))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                    })
            })
            .into_any_element(),
            ChatFormat::Plain => div()
                .text_size(px(CHAT_TEXT_SIZE))
                .text_color(rgb(COLOR_FOREGROUND))
                .child(message.text.clone())
                .into_any_element(),
        };

        let row = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .flex()
            .w_full();

        match message.role {
            ChatRole::Info => row
                .child(
                    div()
                        .w_full()
                        .rounded(px(CHAT_BUBBLE_RADIUS))
                        .border_1()
                        .border_color(rgb(COLOR_BORDER))
                        .bg(rgb(COLOR_CHAT_INFO_BG))
                        .px(px(CHAT_BUBBLE_PADDING))
                        .py(px(SPACE_TIGHT))
                        .text_size(px(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_MUTED))
                        .child(message.text.clone()),
                )
                .into_any_element(),
            ChatRole::User => row
                .justify_end()
                .child(
                    div()
                        .max_w(relative(CHAT_BUBBLE_WIDTH))
                        .rounded(px(CHAT_BUBBLE_RADIUS))
                        .border_1()
                        .border_color(rgb(COLOR_CHAT_USER_BORDER))
                        .bg(rgb(COLOR_CHAT_USER_BG))
                        .p(px(CHAT_BUBBLE_PADDING))
                        .child(content),
                )
                .into_any_element(),
            ChatRole::Agent => row
                .justify_start()
                .child(
                    div()
                        .max_w(relative(CHAT_BUBBLE_WIDTH))
                        .rounded(px(CHAT_BUBBLE_RADIUS))
                        .border_1()
                        .border_color(rgb(COLOR_BORDER))
                        .bg(rgb(COLOR_CHAT_AGENT_BG))
                        .p(px(CHAT_BUBBLE_PADDING))
                        .child(content),
                )
                .into_any_element(),
        }
    }
}

fn code_button_id(code: &str) -> SharedString {
    let mut hasher = DefaultHasher::new();
    code.hash(&mut hasher);
    SharedString::from(format!("chat-copy-{:x}", hasher.finish()))
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let messages: Vec<AnyElement> = self
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| self.render_message(index, message, window, cx))
            .collect();

        div()
            .id("chat-view")
            .debug_selector(|| "chat-view".to_string())
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(COLOR_BASE))
            .text_color(rgb(COLOR_FOREGROUND))
            .child(
                div()
                    .id("chat-log")
                    .debug_selector(|| "chat-log".to_string())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(CHAT_BUBBLE_GAP))
                    .p(px(SPACE_WIDE))
                    .children(messages),
            )
            .when_some(self.status.clone(), |this, status| {
                this.child(
                    div()
                        .id("chat-status")
                        .debug_selector(|| "chat-status".to_string())
                        .px(px(SPACE_WIDE))
                        .pb(px(SPACE_TIGHT))
                        .text_size(px(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_MUTED))
                        .child(status.label()),
                )
            })
            .child(
                div()
                    .id("chat-input")
                    .debug_selector(|| "chat-input".to_string())
                    .px(px(SPACE_WIDE))
                    .pb(px(SPACE_WIDE))
                    .child(Input::new(&self.input).w_full()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cx() -> auro::gpui::TestAppContext {
        let cx = auro::gpui::TestAppContext::single();
        cx.update(|cx| gpui_component::init(cx));
        cx
    }

    #[test]
    fn chat_view_renders_messages_and_composer() {
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_info(
                    "Histórico não estruturado. Envie uma mensagem.".into(),
                    cx,
                );
                chat.push_message(
                    ChatMessage {
                        role: ChatRole::Agent,
                        text: "Olá".into(),
                        format: ChatFormat::Markdown,
                    },
                    cx,
                );
            });
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-message-1").is_some());
        assert!(vcx.debug_bounds("chat-input").is_some());
    }
}
