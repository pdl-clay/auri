#!/usr/bin/env bash
# Golden-image regression for the cross-workspace connect gesture.
#
#   ./scripts/visual-cross-workspace.sh --record   # write goldens (once)
#   ./scripts/visual-cross-workspace.sh --check    # compare against goldens
#
# Uses the same isolated window on a secondary output and the same virtual
# pointer as visual-regression.sh. The fixture has two workspaces whose agents
# share id 1 and overlap: a normal agent must be able to connect across them,
# and the proxy must stay visible instead of being painted under the card.
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
APP="$ROOT/target/debug/canvas"
POINTER_MANIFEST="$VISUAL/pointer/Cargo.toml"
POINTER_TARGET="$ROOT/target/visual-pointer"
POINTER="$POINTER_TARGET/debug/wlr-virtual-pointer"
STATES=(cross-workspace-proxy-target cross-workspace-proxy-source)

die() { printf 'visual-cross-workspace: %s\n' "$*" >&2; exit 1; }

if [[ "$MODE" == --check ]]; then
    for state in "${STATES[@]}"; do
        [[ -f "$GOLDEN/$state.png" ]] || die "missing golden $GOLDEN/$state.png"
    done
else
    for state in "${STATES[@]}"; do
        if [[ -e "$GOLDEN/$state.png" ]]; then
            die "golden $GOLDEN/$state.png already exists (remove it to re-record)"
        fi
    done
fi

for tool in niri grim magick jq cargo; do
    command -v "$tool" >/dev/null || die "$tool is required"
done
[[ -x "$APP" ]] || die "missing debug app $APP (run: cargo build -p canvas)"

cargo build --offline --manifest-path "$POINTER_MANIFEST" --target-dir "$POINTER_TARGET" >&2 \
    || die "cannot build pointer helper"

TMP="$(mktemp -d)"
APP_PID=''
ORIGINAL_FOCUS="$(niri msg --json windows 2>/dev/null | jq -r '.[] | select(.is_focused == true) | .id' | head -1 || true)"
BASELINE="$(niri msg --json windows 2>/dev/null | jq -c '[.[] | select(.app_id == "com.example.canvas") | .id]')"
ACTIVE_OUTPUT="$(niri msg --json focused-output 2>/dev/null | jq -r '.name' || true)"
OUTPUT="${VISUAL_OUTPUT:-$(niri msg --json outputs 2>/dev/null | jq -r --arg active "$ACTIVE_OUTPUT" 'keys[] | select(. != $active)' | head -1)}"
[[ -n "$OUTPUT" && "$OUTPUT" != null ]] || die "no secondary output (set VISUAL_OUTPUT)"

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
cat > "$TMP/config/auro-canvas/workspaces.json" <<'JSON'
{"active":1,"next_id":3,"custom_agents":[{"id":1,"name":"Agente A","command":"printf 'agente A pronto\\n'; exec bash","icon":"Claude"},{"id":2,"name":"Agente B","command":"printf 'agente B pronto\\n'; exec bash","icon":"Codex"}],"next_custom_id":3}
{"id":1,"name":"Auri","icon":"Star","root":"/tmp","color":"Brass","invalid_root":false,"canvas":{"terminals":[],"notes":[],"agents":[{"id":1,"kind":{"Custom":1},"name":"Agente A","bounds":{"x":80.0,"y":80.0,"width":420.0,"height":260.0},"admin":false}],"connections":[],"proxy_positions":{},"selected":1,"next_id":2},"unloaded":false}
{"id":2,"name":"Auro","icon":"Folder","root":"/tmp","color":"Brass","invalid_root":false,"canvas":{"terminals":[],"notes":[],"agents":[{"id":1,"kind":{"Custom":2},"name":"Agente B","bounds":{"x":80.0,"y":80.0,"width":420.0,"height":260.0},"admin":false}],"connections":[],"proxy_positions":{},"selected":null,"next_id":2},"unloaded":false}
JSON

(cd "$ROOT" && exec env -u WAYLAND_DISPLAY \
    XDG_CONFIG_HOME="$TMP/config" XDG_CACHE_HOME="$TMP/cache" XDG_RUNTIME_DIR="$TMP/runtime" \
    "$APP" >"$TMP/app.log" 2>&1) &
APP_PID=$!

WINDOW=''
for _ in $(seq 1 80); do
    WINDOW="$(niri msg --json windows 2>/dev/null | jq -r --argjson base "$BASELINE" '.[] | select(.app_id == "com.example.canvas" and (.id as $id | $base | index($id) | not)) | .id' | head -1 || true)"
    [[ -n "$WINDOW" && "$WINDOW" != null ]] && break
    kill -0 "$APP_PID" 2>/dev/null || die "app exited before creating a window: $(tail -5 "$TMP/app.log")"
    sleep 0.25
done
[[ -n "$WINDOW" && "$WINDOW" != null ]] || die "no new niri window"

niri msg action focus-window --id "$WINDOW" >/dev/null
niri msg action move-window-to-monitor "$OUTPUT" >/dev/null
sleep 0.5
niri msg action fullscreen-window >/dev/null
for _ in $(seq 1 20); do
    niri msg --json windows 2>/dev/null | jq -e --argjson id "$WINDOW" '.[] | select(.id == $id and .is_focused)' >/dev/null && break
    niri msg action focus-window --id "$WINDOW" >/dev/null 2>&1 || true
    sleep 0.1
done
sleep 2

read -r WIDTH HEIGHT < <(niri msg --json outputs 2>/dev/null | jq -r --arg output "$OUTPUT" '.[$output].logical | [.width, .height] | @tsv')
[[ "$WIDTH" -gt 0 && "$HEIGHT" -gt 0 ]] || die "bad output geometry: ${WIDTH}x${HEIGHT}"
EXTENT="${WIDTH}x${HEIGHT}"

click() {
    niri msg action focus-window --id "$WINDOW" >/dev/null 2>&1 || true
    "$POINTER" click "$1" "$2" "$3" --extent "$EXTENT" --output "$OUTPUT"
    sleep 0.8
}
capture() {
    grim -o "$OUTPUT" "$TMP/screen.png"
    magick "$TMP/screen.png" -crop "${WIDTH}x${HEIGHT}+0+0" +repage "$TMP/$1.png"
}

# 1. Load Auri, select the normal agent, press the island bolt.
click 112 121 left
sleep 2
click 500 96 left
click "$((WIDTH - 30))" "$((HEIGHT / 2 + 48))" left

# 2. The drawer is the workspace step; Auro opens with the target armed.
click 112 143 left
sleep 2
click 500 96 left
sleep 1
grep -q '"from_workspace":1' "$TMP/config/auro-canvas/workspaces.json" \
    || die "the click did not create a cross-workspace edge: $(cat "$TMP/config/auro-canvas/workspaces.json")"
capture cross-workspace-proxy-target

# 3. The same edge must produce a proxy when the source workspace is reopened.
click 112 121 left
sleep 2
capture cross-workspace-proxy-source

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
    awk -v diff="$DIFF" -v limit="$LIMIT" 'BEGIN { exit !((diff + 0) <= (limit + 0)) }' \
        || die "$state differs in $DIFF pixels (limit $LIMIT)"
    printf 'ok %s (%s/%s differing pixels)\n' "$state" "$DIFF" "$LIMIT"
done
