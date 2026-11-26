#!/usr/bin/env bash
# The installer must resolve the latest release through the redirecting asset
# URLs, not the unauthenticated GitHub API (60 requests/hour per IP).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURES="$ROOT/tests/fixtures/installer"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin" "$TMP/install"

cat > "$TMP/bin/curl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
url=''
out=''
while (( $# )); do
    case "$1" in
        -o) out="$2"; shift 2 ;;
        -*) shift ;;
        *) url="$1"; shift ;;
    esac
done
printf '%s\n' "$url" >> "$LOG"
case "$url" in
    *api.github.com*) exit 22 ;;
    */auro-update.json) cp "$FIXTURES/auro-update.json" "$out" ;;
    */auri-cli.json) cp "$FIXTURES/auri-cli.json" "$out" ;;
    */fake.AppImage) cp "$FIXTURES/fake.AppImage" "$out" ;;
    */fake-cli) cp "$FIXTURES/fake-cli" "$out" ;;
    */icon.png) : > "$out" ;;
    *) printf 'unexpected url: %s\n' "$url" >&2; exit 22 ;;
esac
STUB
chmod +x "$TMP/bin/curl"

export FIXTURES
export LOG="$TMP/urls.log"
: > "$LOG"

PATH="$TMP/bin:$PATH" bash "$ROOT/scripts/install-appimage.sh" \
    --dir "$TMP/install" --no-desktop >"$TMP/out" 2>&1 || {
    echo 'installer failed:'
    cat "$TMP/out"
    exit 1
}

if grep -q 'api.github.com' "$LOG"; then
    echo 'regression: installer consulted the rate-limited GitHub API'
    cat "$LOG"
    exit 1
fi
[[ -x "$TMP/install/auri-canvas" ]] || { echo 'AppImage not installed'; exit 1; }
[[ -x "$TMP/install/auri" ]] || { echo 'CLI not installed'; exit 1; }
echo 'ok: installer resolved latest release without the GitHub API'
