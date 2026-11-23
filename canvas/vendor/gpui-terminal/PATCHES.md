# Local patches

## `gpui-terminal` 0.1.0

Vendored from crates.io to remove the per-character shaping on every paint.

- `render.rs`: a per-row cache now stores shaped text runs and merged
  background rectangles. Rows are rebuilt only when alacritty reports damage
  (`Term::damage` / `Term::reset_damage`), and text is batched into runs with
  identical styling, so a row costs one shaping call per style change instead
  of one per character. Cell metrics are measured once per font instead of
  every frame.
- `view.rs`: a font change (canvas zoom) marks the row cache dirty and starts a
  150 ms settle window. The PTY is not resized while the font is settling, so a
  zoom gesture no longer makes the child application redraw its whole screen
  on every scroll tick.

Upstream: https://github.com/zortax/gpui-terminal (MIT OR Apache-2.0).
