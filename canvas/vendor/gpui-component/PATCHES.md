# Local patch — gpui-component 0.5.1

Vendored from crates.io to allow a per-Input text colour. Upstream 0.5.1
hardcodes `cx.theme().foreground` in `input/element.rs`, which made sticky-note
text white on a light note background.

- `src/input/state.rs`: added `pub text_color: Option<gpui::Hsla>` to
  `InputState`, defaulting to `None`.
- `src/input/element.rs`: editor text uses `state.text_color` when set,
  otherwise the theme foreground.

Auri sets this to `COLOR_NOTE_FG` for sticky notes only. All other inputs keep
the theme foreground.

Upstream: https://github.com/longbridge/gpui-kit (MIT/Apache-2.0).
