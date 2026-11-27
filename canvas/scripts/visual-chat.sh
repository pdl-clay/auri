#!/usr/bin/env bash
# Deterministic golden-image suite for the agent chat view.
#
#   ./scripts/visual-chat.sh --record   # write goldens
#   ./scripts/visual-chat.sh --check    # compare against goldens
set -euo pipefail

MODE=''
case "${1:-}" in
    --record|--check) MODE="$1" ;;
    *) echo 'usage: visual-chat.sh --record|--check' >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VISUAL="$ROOT/tests/visual"
GOLDEN="$VISUAL/golden"
FIXTURE="$VISUAL/fixture/agent-chat"
APP="$ROOT/target/debug/canvas"
POINTER_MANIFEST="$VISUAL/pointer/Cargo.toml"
POINTER_TARGET="$ROOT/target/visual-pointer"
POINTER="$POINTER_TARGET/debug/wlr-virtual-pointer"
STATES=(agent-chat agent-chat-info agent-chat-narrow agent-chat-ask agent-chat-scroll agent-chat-scroll-bottom agent-chat-models agent-chat-models-top)

die() { printf 'visual-chat: %s\n' "$*" >&2; exit 1; }

for tool in niri grim magick jq cargo; do
    command -v "$tool" >/dev/null || die "$tool is required"
done

cargo build --offline -p canvas >&2
cargo build --offline --manifest-path "$POINTER_MANIFEST" --target-dir "$POINTER_TARGET" >&2
[[ -x "$APP" ]] || die "missing $APP"
[[ -x "$POINTER" ]] || die "missing pointer helper $POINTER"
[[ -f "$FIXTURE/workspaces.json" ]] || die "missing chat fixture"
[[ -f "$FIXTURE/info.json" ]] || die "missing info fixture"
[[ -f "$FIXTURE/narrow.json" ]] || die "missing narrow fixture"
[[ -f "$FIXTURE/session.jsonl" ]] || die "missing session fixture"
[[ -f "$FIXTURE/session-long.jsonl" ]] || die "missing long session fixture"
[[ -f "$FIXTURE/session-ask.jsonl" ]] || die "missing ask session fixture"

ORIGINAL_FOCUS="$(niri msg --json windows 2>/dev/null | jq -r '.[] | select(.is_focused == true) | .id' | head -1 || true)"
ACTIVE_OUTPUT="$(niri msg --json focused-output 2>/dev/null | jq -r '.name' || true)"
OUTPUT="${VISUAL_OUTPUT:-$(niri msg --json outputs 2>/dev/null | jq -r --arg active "$ACTIVE_OUTPUT" 'keys[] | select(. != $active)' | head -1)}"
[[ -n "$OUTPUT" && "$OUTPUT" != null ]] || die "no secondary output (set VISUAL_OUTPUT)"
read -r WIDTH HEIGHT < <(niri msg --json outputs 2>/dev/null | jq -r --arg output "$OUTPUT" '.[$output].logical | [.width, .height] | @tsv')
[[ "$WIDTH" -gt 0 && "$HEIGHT" -gt 0 ]] || die "bad output geometry for $OUTPUT"
EXTENT="${WIDTH}x${HEIGHT}"

TMP="$(mktemp -d)"
APP_PID=''
WINDOW=''
cleanup() {
    if [[ -n "$WINDOW" ]]; then
        niri msg action focus-window --id "$WINDOW" >/dev/null 2>&1 || true
        niri msg action close-window >/dev/null 2>&1 || true
    fi
    if [[ -n "$APP_PID" ]]; then
        kill "$APP_PID" 2>/dev/null || true
        wait "$APP_PID" 2>/dev/null || true
    fi
    if [[ -n "$ORIGINAL_FOCUS" && "$ORIGINAL_FOCUS" != null ]]; then
        niri msg action focus-window --id "$ORIGINAL_FOCUS" >/dev/null 2>&1 || true
    fi
    rm -rf "$TMP"
}
trap cleanup EXIT

# Deterministic skill list and model catalog for the states that open the
# slash picker or the model menu. XDG paths are already isolated; HOME only
# feeds skill discovery and the `omp models` lookup.
HOME_FIXTURE="$TMP/home"
for skill in alpha beta gamma; do
    mkdir -p "$HOME_FIXTURE/.agents/skills/$skill"
    printf -- '---\nname: %s\ndescription: Skill de teste %s\n---\n' "$skill" "$skill" \
        > "$HOME_FIXTURE/.agents/skills/$skill/SKILL.md"
done
mkdir -p "$HOME_FIXTURE/.local/bin"
{
    printf '{"models":['
    # The session's current model, named so the catalog sorts it after the
    # generic rows: the menu must move it to the top on its own.
    printf '{"provider":"opencode-go","selector":"opencode-go/deepseek-v4.1-flash","name":"DeepSeek V4.1 Flash","contextWindow":1000000}'
    for provider in opencode-go alpha beta gamma; do
        for n in $(seq -w 1 15); do
            printf ','
            printf '{"provider":"%s","selector":"%s/model-%s","name":"Alpha %s %s"}' \
                "$provider" "$provider" "$n" "$provider" "$n"
        done
    done
    printf ']}\n'
} > "$HOME_FIXTURE/.local/models.json"
cat > "$HOME_FIXTURE/.local/bin/omp" <<'EOF'
#!/usr/bin/env bash
cat "$(dirname "$0")/../models.json"
EOF
chmod +x "$HOME_FIXTURE/.local/bin/omp"

capture() {
    local state="$1"
    grim -o "$OUTPUT" "$TMP/screen.png"
    magick "$TMP/screen.png" -crop "${WIDTH}x${HEIGHT}+0+0" +repage "$TMP/$state.png"
}

click() {
    "$POINTER" click "$1" "$2" "$3" --extent "$EXTENT" --output "$OUTPUT"
    sleep 0.35
}

scroll() {
    "$POINTER" scroll "$1" "$2" "$3" --extent "$EXTENT" --output "$OUTPUT"
    sleep 0.35
}

# Gestures that set up each state after the fixture workspace is selected.
interact() {
    case "$1" in
        agent-chat-scroll)
            # The long transcript opens at the bottom; the wheel must walk it
            # to the first message without moving the card.
            for _ in $(seq 1 8); do scroll 500 400 down; done
            ;;
        agent-chat-models)
            click 1100 130 left
            sleep 1.5
            # Scroll the model catalog to its last row; the card must stay put.
            for _ in $(seq 1 5); do scroll 950 300 up; done
            ;;
        agent-chat-models-top)
            # Same menu without scrolling: the current model leads its provider.
            click 1100 130 left
            sleep 1.5
            ;;
    esac
}

run_state() {
    local state="$1" fixture="$2" with_session="$3" session_file="${4:-session.jsonl}"
    rm -rf "$TMP/config" "$TMP/cache" "$TMP/runtime"
    mkdir -p "$TMP/config/auro-canvas" "$TMP/cache" "$TMP/runtime"
    chmod 700 "$TMP/runtime"
    if [[ "$with_session" == true ]]; then
        cp "$FIXTURE/$session_file" "$TMP/session.jsonl"
        jq -c --arg session "$TMP/session.jsonl" \
            'if .canvas? then .canvas.agents[0].session = $session else . end' \
            "$fixture" > "$TMP/config/auro-canvas/workspaces.json"
    else
        cp "$fixture" "$TMP/config/auro-canvas/workspaces.json"
    fi

    local baseline
    baseline="$(niri msg --json windows 2>/dev/null | jq -c '[.[] | select(.app_id == "com.example.canvas") | .id]')"
    env -u WAYLAND_DISPLAY HOME="$HOME_FIXTURE" PATH="$HOME_FIXTURE/.local/bin:$PATH" \
        XDG_CONFIG_HOME="$TMP/config" XDG_CACHE_HOME="$TMP/cache" \
        XDG_RUNTIME_DIR="$TMP/runtime" \
        "$APP" >"$TMP/$state.log" 2>&1 &
    APP_PID=$!
    WINDOW=''
    for _ in $(seq 1 80); do
        WINDOW="$(niri msg --json windows 2>/dev/null | jq -r --argjson base "$baseline" '.[] | select(.app_id == "com.example.canvas" and (.id as $id | $base | index($id) | not)) | .id' | head -1 || true)"
        [[ -n "$WINDOW" && "$WINDOW" != null ]] && break
        sleep 0.1
    done
    [[ -n "$WINDOW" && "$WINDOW" != null ]] || die "no window for $state: $(tail -5 "$TMP/$state.log")"

    niri msg action focus-window --id "$WINDOW" >/dev/null
    niri msg action move-window-to-monitor "$OUTPUT" >/dev/null
    sleep 0.4
    niri msg action focus-window --id "$WINDOW" >/dev/null
    niri msg action fullscreen-window >/dev/null
    sleep 2
    "$POINTER" click 112 121 left --extent "$EXTENT" --output "$OUTPUT"
    sleep 3
    interact "$state"
    capture "$state"

    niri msg action close-window >/dev/null 2>&1 || true
    kill "$APP_PID" 2>/dev/null || true
    wait "$APP_PID" 2>/dev/null || true
    APP_PID=''
    WINDOW=''
    rm -f "$TMP/session.jsonl"
}

for state in "${STATES[@]}"; do
    case "$state" in
        agent-chat) run_state "$state" "$FIXTURE/workspaces.json" true ;;
        agent-chat-info) run_state "$state" "$FIXTURE/info.json" false ;;
        agent-chat-narrow) run_state "$state" "$FIXTURE/narrow.json" true ;;
        agent-chat-ask) run_state "$state" "$FIXTURE/workspaces.json" true session-ask.jsonl ;;
        agent-chat-scroll) run_state "$state" "$FIXTURE/workspaces.json" true session-long.jsonl ;;
        agent-chat-scroll-bottom) run_state "$state" "$FIXTURE/workspaces.json" true session-long.jsonl ;;
        agent-chat-models) run_state "$state" "$FIXTURE/workspaces.json" true ;;
        agent-chat-models-top) run_state "$state" "$FIXTURE/workspaces.json" true ;;
    esac
done

if [[ "$MODE" == --record ]]; then
    mkdir -p "$GOLDEN"
    for state in "${STATES[@]}"; do
        if [[ -f "$GOLDEN/$state.png" && "${FORCE_GOLDENS:-0}" != 1 ]]; then
            die "golden $GOLDEN/$state.png exists (set FORCE_GOLDENS=1)"
        fi
        cp "$TMP/$state.png" "$GOLDEN/$state.png"
        printf 'recorded %s (%s)\n' "$state" "$GOLDEN/$state.png"
    done
    exit 0
fi

for state in "${STATES[@]}"; do
    [[ -f "$GOLDEN/$state.png" ]] || die "missing golden $GOLDEN/$state.png"
    DIFF="$(magick compare -metric AE "$GOLDEN/$state.png" "$TMP/$state.png" null: 2>&1 || true)"
    TOTAL="$(magick identify -format '%[fx:w*h]' "$GOLDEN/$state.png")"
    LIMIT="$(awk -v total="$TOTAL" 'BEGIN { printf "%d", total * 0.005 }')"
    awk -v diff="$DIFF" -v limit="$LIMIT" 'BEGIN { exit !((diff + 0) <= (limit + 0)) }' \
        || die "$state differs in $DIFF pixels (limit $LIMIT)"
    printf 'ok %s (%s/%s differing pixels)\n' "$state" "$DIFF" "$LIMIT"
done
