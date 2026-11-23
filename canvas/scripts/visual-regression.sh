#!/usr/bin/env bash
# Deterministic golden-image suite for the AURI canvas.
#
#   ./scripts/visual-regression.sh --record   # write goldens (once)
#   ./scripts/visual-regression.sh --check    # compare against goldens
#
# The app runs with an isolated config/cache/runtime, in its own fullscreen
# niri window. Clicks go through the wlr-virtual-pointer helper, so the same
# three states are reached on every run. FORCE_GOLDENS=1 allows re-recording.
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
STATES=(empty-canvas connected-cards edit-dialog)

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
niri msg action fullscreen-window >/dev/null
sleep 1.5

OUTPUT="$(niri msg --json focused-output 2>/dev/null | jq -r '.name')"
[[ -n "$OUTPUT" && "$OUTPUT" != null ]] || die "no focused niri output"
read -r WIDTH HEIGHT < <(niri msg --json focused-window 2>/dev/null | jq -r '[.layout.window_size[0], .layout.window_size[1]] | @tsv')
[[ "$WIDTH" -gt 0 && "$HEIGHT" -gt 0 ]] || die "bad window geometry ${WIDTH}x${HEIGHT}"
EXTENT="${WIDTH}x${HEIGHT}"

click() {
    local x="$1" y="$2" button="$3"
    "$POINTER" click "$x" "$y" "$button" --extent "$EXTENT"
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

capture empty-canvas
click "$ROW_X" "$ROW_Y" left
sleep 2
capture connected-cards
click "$CARD_X" "$CARD_Y" right
click "$EDIT_X" "$EDIT_Y" left
sleep 1
capture edit-dialog

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
    awk -v diff="$DIFF" -v limit="$LIMIT" 'BEGIN { exit !(diff <= limit) }' \
        || die "$state differs in $DIFF pixels (limit $LIMIT)"
    printf 'ok %s (%s/%s differing pixels)\n' "$state" "$DIFF" "$LIMIT"
done
