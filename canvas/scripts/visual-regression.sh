#!/usr/bin/env bash
# Deterministic golden-image suite for the AURI canvas.
#
#   ./scripts/visual-regression.sh --record   # write goldens (once)
#   ./scripts/visual-regression.sh --check    # compare against goldens
#
# The app runs with an isolated config/cache/runtime, in its own fullscreen
# niri window on a screen you are not using: the output is pinned so the run is
# deterministic and does not cover the active monitor. Set VISUAL_OUTPUT to
# choose a screen explicitly. Clicks go through the wlr-virtual-pointer helper,
# so the same states are reached on every run. FORCE_GOLDENS=1 allows
# re-recording.
set -euo pipefail

usage() {
    printf 'usage: %s --record|--check\n' "$0" >&2
    exit 2
}

MODE=''
case "${1:-}" in
    --record|--check) MODE="$1" ;;
    *) usage ;;
esac
[[ $# -eq 1 ]] || usage

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VISUAL="$ROOT/tests/visual"
GOLDEN="$VISUAL/golden"
FIXTURE="$VISUAL/fixture/auro-canvas/workspaces.json"
APP="$ROOT/target/debug/canvas"
POINTER_MANIFEST="$VISUAL/pointer/Cargo.toml"
POINTER_TARGET="$ROOT/target/visual-pointer"
POINTER="$POINTER_TARGET/debug/wlr-virtual-pointer"
STATES=(empty-canvas connected-cards terminal-scroll terminal-thumb-drag edit-dialog note-connect connect-menu connection-popover)

die() { printf 'visual-regression: %s\n' "$*" >&2; exit 1; }

if [[ "$MODE" == --check ]]; then
    for state in "${STATES[@]}"; do
        [[ -f "$GOLDEN/$state.png" ]] || die "missing golden $GOLDEN/$state.png"
    done
else
    for state in "${STATES[@]}"; do
        if [[ -e "$GOLDEN/$state.png" && "${FORCE_GOLDENS:-0}" != 1 ]]; then
            die "golden $GOLDEN/$state.png already exists (set FORCE_GOLDENS=1 to replace)"
        fi
    done
fi

for tool in niri grim magick jq cargo; do
    command -v "$tool" >/dev/null || die "$tool is required"
done
[[ -f "$FIXTURE" ]] || die "missing fixture $FIXTURE"
[[ -f "$POINTER_MANIFEST" ]] || die "missing virtual pointer helper $POINTER_MANIFEST"
[[ -x "$APP" ]] || die "missing debug app $APP (run: cargo build -p canvas)"


cargo build --offline --manifest-path "$POINTER_MANIFEST" --target-dir "$POINTER_TARGET" >&2 || die "cannot build pointer helper"
[[ -x "$POINTER" ]] || die "pointer helper was not built at $POINTER"

TMP="$(mktemp -d)"
APP_PID=''
ORIGINAL_FOCUS="$(niri msg --json windows 2>/dev/null | jq -r '.[] | select(.is_focused == true) | .id' | head -1 || true)"
BASELINE="$(niri msg --json windows 2>/dev/null | jq -c '[.[] | select(.app_id == "com.example.canvas") | .id]')"
# Never put the test window on the monitor the user is looking at.
ACTIVE_OUTPUT="$(niri msg --json focused-output 2>/dev/null | jq -r '.name' || true)"
OUTPUT="${VISUAL_OUTPUT:-$(niri msg --json outputs 2>/dev/null | jq -r --arg active "$ACTIVE_OUTPUT" 'keys[] | select(. != $active)' | head -1)}"
[[ -n "$OUTPUT" && "$OUTPUT" != null ]] || die "no secondary output for the visual suite (set VISUAL_OUTPUT)"
cleanup() {
    if [[ -n "$APP_PID" ]]; then
        kill "$APP_PID" 2>/dev/null || true
        wait "$APP_PID" 2>/dev/null || true
    fi
    if [[ -n "$ORIGINAL_FOCUS" ]]; then
        niri msg action focus-window --id "$ORIGINAL_FOCUS" >/dev/null 2>&1 || true
    fi
    rm -rf "$TMP"
}
trap cleanup EXIT

mkdir -p "$TMP/config/auro-canvas" "$TMP/cache" "$TMP/runtime"
chmod 700 "$TMP/runtime"
cp "$FIXTURE" "$TMP/config/auro-canvas/workspaces.json"

env -u WAYLAND_DISPLAY XDG_CONFIG_HOME="$TMP/config" XDG_CACHE_HOME="$TMP/cache" XDG_RUNTIME_DIR="$TMP/runtime" \
    "$APP" >"$TMP/app.log" 2>&1 &
APP_PID=$!

WINDOW=''
for _ in $(seq 1 80); do
    WINDOW="$(niri msg --json windows 2>/dev/null | jq -r --argjson base "$BASELINE" '.[] | select(.app_id == "com.example.canvas" and (.id as $id | $base | index($id) | not)) | .id' | head -1 || true)"
    [[ -n "$WINDOW" && "$WINDOW" != null ]] && break
    kill -0 "$APP_PID" 2>/dev/null || die "app exited before creating a window: $(tail -5 "$TMP/app.log")"
    sleep 0.25
done
[[ -n "$WINDOW" && "$WINDOW" != null ]] || die "no new niri window for app pid $APP_PID"

niri msg action focus-window --id "$WINDOW" >/dev/null
niri msg action move-window-to-monitor "$OUTPUT" >/dev/null
sleep 0.5
niri msg action fullscreen-window >/dev/null
# Moving between outputs can drop focus; a lost first click would freeze the
# state and make every later golden the empty canvas.
for _ in $(seq 1 20); do
    if niri msg --json windows 2>/dev/null | jq -e --argjson id "$WINDOW" '.[] | select(.id == $id and .is_focused)' >/dev/null; then
        break
    fi
    niri msg action focus-window --id "$WINDOW" >/dev/null 2>&1 || true
    sleep 0.1
done
sleep 2

read -r WIDTH HEIGHT < <(niri msg --json outputs 2>/dev/null | jq -r --arg output "$OUTPUT" '.[$output].logical | [.width, .height] | @tsv')
[[ "$WIDTH" -gt 0 && "$HEIGHT" -gt 0 ]] || die "bad output geometry for $OUTPUT: ${WIDTH}x${HEIGHT}"
printf 'visual-regression: using output %s (%sx%s)\n' "$OUTPUT" "$WIDTH" "$HEIGHT" >&2
EXTENT="${WIDTH}x${HEIGHT}"

click() {
    local x="$1" y="$2" button="$3"
    "$POINTER" click "$x" "$y" "$button" --extent "$EXTENT" --output "$OUTPUT"
    sleep 0.35
}

scroll() {
    local x="$1" y="$2" direction="$3"
    "$POINTER" scroll "$x" "$y" "$direction" --extent "$EXTENT" --output "$OUTPUT"
    sleep 0.35
}

drag() {
    local x1="$1" y1="$2" x2="$3" y2="$4"
    "$POINTER" drag "$x1" "$y1" "$x2" "$y2" --extent "$EXTENT" --output "$OUTPUT"
    sleep 0.35
}

capture() {
    local state="$1"
    grim -o "$OUTPUT" "$TMP/screen.png"
    magick "$TMP/screen.png" -crop "${WIDTH}x${HEIGHT}+0+0" +repage "$TMP/$state.png"
}

# Viewport-relative coordinates, tuned against the fullscreen fixture window.
ROW_X="${ROW_X:-112}"
ROW_Y="${ROW_Y:-121}"
CARD_X="${CARD_X:-400}"
CARD_Y="${CARD_Y:-180}"
EDIT_X="${EDIT_X:-470}"
EDIT_Y="${EDIT_Y:-200}"
CANCEL_X="${CANCEL_X:-956}"
CANCEL_Y="${CANCEL_Y:-721}"
# The island is centred vertically; with the fixture's selected card it holds
# four 28px buttons separated by 4px, so their centres sit at -48/-16/+16/+48.
ISLAND_X="${ISLAND_X:-$((WIDTH - 30))}"
NOTE_TOOL_Y="${NOTE_TOOL_Y:-$((HEIGHT / 2 + 16))}"
BOLT_Y="${BOLT_Y:-$((HEIGHT / 2 + 48))}"
NOTE_SPOT_X="${NOTE_SPOT_X:-700}"
NOTE_SPOT_Y="${NOTE_SPOT_Y:-700}"

capture empty-canvas
click "$ROW_X" "$ROW_Y" left
sleep 2
capture connected-cards

# Scrollback regressions: wheel up over the terminal body must move its history
# without moving the canvas; dragging the sidebar thumb must not resize the card.
scroll "$CARD_X" "$CARD_Y" up
scroll "$CARD_X" "$CARD_Y" up
scroll "$CARD_X" "$CARD_Y" up
sleep 0.5
capture terminal-scroll
drag 720 250 720 130
sleep 0.5
capture terminal-thumb-drag
click "$CARD_X" "$CARD_Y" right
click "$EDIT_X" "$EDIT_Y" left
sleep 1
capture edit-dialog
click "$CANCEL_X" "$CANCEL_Y" left
sleep 0.5
# Regression state: place a note, select it, then press the bolt. The bolt
# press must keep the selection and enter connection mode; if the press clears
# the selection first, the bolt disappears and this golden diverges.
click "$ISLAND_X" "$NOTE_TOOL_Y" left
click "$NOTE_SPOT_X" "$NOTE_SPOT_Y" left
click "$NOTE_SPOT_X" "$((NOTE_SPOT_Y - 96))" left
sleep 0.5
click "$ISLAND_X" "$BOLT_Y" left
sleep 1
capture note-connect

# Last state: leave connection mode, press the rope between the two fixture
# cards and capture the inspector with both endpoints and its remove button.
ROPE_X="${ROPE_X:-932}"
ROPE_Y="${ROPE_Y:-464}"
EMPTY_X="${EMPTY_X:-1200}"
EMPTY_Y="${EMPTY_Y:-880}"
click "$EMPTY_X" "$EMPTY_Y" left

# Right-click the note title: the local menu offers the same Conectar gesture.
click "$NOTE_SPOT_X" "$((NOTE_SPOT_Y - 96))" right
sleep 0.5
capture connect-menu
click "$EMPTY_X" "$EMPTY_Y" left
sleep 0.5
click "$ROPE_X" "$ROPE_Y" left
sleep 0.5
capture connection-popover

if [[ "$MODE" == --record ]]; then
    mkdir -p "$GOLDEN"
    for state in "${STATES[@]}"; do
        cp "$TMP/$state.png" "$GOLDEN/$state.png"
        printf 'recorded %s (%s)\n' "$state" "$GOLDEN/$state.png"
    done
    exit 0
fi

for state in "${STATES[@]}"; do
    DIFF="$(magick compare -metric AE "$GOLDEN/$state.png" "$TMP/$state.png" null: 2>&1 || true)"
    TOTAL="$(magick identify -format '%[fx:w*h]' "$GOLDEN/$state.png")"
    LIMIT="$(awk -v total="$TOTAL" 'BEGIN { printf "%d", total * 0.005 }')"
    # Force numeric comparison: the raw metric is "47.485 (2.3e-05)", and awk
    # otherwise compares the leading string lexicographically ("47" > "10").
    awk -v diff="$DIFF" -v limit="$LIMIT" 'BEGIN { exit !((diff + 0) <= (limit + 0)) }' \
        || die "$state differs in $DIFF pixels (limit $LIMIT)"
    printf 'ok %s (%s/%s differing pixels)\n' "$state" "$DIFF" "$LIMIT"
done
