#!/usr/bin/env bash
# Instala AppImages no Linux. Sem argumentos, instala a release estável mais
# recente do Auri, verificando manifestos assinados antes de tocar nos arquivos.
set -euo pipefail

readonly REPO='pdl-clay/auri'
readonly PUBLIC_KEY='d+Ltt6kOIF9kp5raaYtZTOEnZ1r3NjJr/D+UE8vKUi8='
readonly API="https://api.github.com/repos/${REPO}/releases/latest"

die() { printf 'erro: %s\n' "$*" >&2; exit 1; }
warn() { printf 'aviso: %s\n' "$*" >&2; }

usage() {
    cat <<'EOF'
Uso:
  install-appimage.sh [opções] [ARQUIVO|URL]

Sem argumentos, instala a release estável mais recente do Auri: verifica a
assinatura Ed25519 dos manifestos, o tamanho e o SHA-256 de cada artefato,
instala o AppImage e o CLI `auri` em ~/.local/bin e cria a entrada .desktop.

Opções:
  --manifest ARQUIVO   manifesto Auro assinado (URL ou arquivo local)
  --sha256 HASH        exigir este SHA-256 quando não houver manifesto
  --key CHAVE          chave pública Ed25519 em base64 (padrão: a do Auri)
  --name NOME          nome exibido no desktop (padrão: Auri)
  --exec NOME          nome do AppImage instalado (padrão: auri-canvas)
  --dir DIR            destino dos executáveis (padrão: $XDG_BIN_HOME ou ~/.local/bin)
  --icon ARQUIVO       ícone da entrada .desktop (URL ou arquivo local)
  --no-desktop         não criar entrada .desktop
  --no-cli             não instalar o CLI `auri` (somente modo Auri)
  -h, --help           mostra esta ajuda

Exemplos:
  install-appimage.sh
  install-appimage.sh ./outro.AppImage --name Outro --exec outro
  install-appimage.sh --manifest auro-update.json
  install-appimage.sh https://site/app.AppImage --sha256 <hash>
EOF
}

need_arg() { (( $# >= 2 )) || die "$1 precisa de um valor"; }
need_tools() {
    local tool
    for tool in curl jq openssl base64 sha256sum awk dd install mktemp uname; do
        command -v "$tool" >/dev/null 2>&1 || die "ferramenta obrigatória ausente: $tool"
    done
}

source=''
manifest=''
sha256=''
key="$PUBLIC_KEY"
name=''
exec_name=''
install_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
icon=''
desktop=true
cli=true

while (( $# )); do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        --manifest) need_arg "$@"; manifest="$2"; shift 2 ;;
        --sha256) need_arg "$@"; sha256="$2"; shift 2 ;;
        --key) need_arg "$@"; key="$2"; shift 2 ;;
        --name) need_arg "$@"; name="$2"; shift 2 ;;
        --exec) need_arg "$@"; exec_name="$2"; shift 2 ;;
        --dir) need_arg "$@"; install_dir="$2"; shift 2 ;;
        --icon) need_arg "$@"; icon="$2"; shift 2 ;;
        --no-desktop) desktop=false; shift ;;
        --no-cli) cli=false; shift ;;
        -*) die "opção desconhecida: $1" ;;
        *)
            [[ -z "$source" ]] || die "muitos argumentos: $1"
            source="$1"; shift
            ;;
    esac
done

need_tools
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fetch() {
    curl -fL --retry 3 --retry-delay 1 --connect-timeout 10 \
        -o "$2" "$1" || die "falha ao baixar $1"
}

local_or_url() {
    # $1 = origem, $2 = destino
    case "$1" in
        http://*|https://*) fetch "$1" "$2" ;;
        *) [[ -f "$1" ]] || die "arquivo não encontrado: $1"; cp -- "$1" "$2" ;;
    esac
}

manifest_field() {
    jq -r "$2 // empty" "$1"
}

verify_manifest() {
    local file="$1" format platform expected signature
    jq -e 'type == "object"' "$file" >/dev/null 2>&1 \
        || die "manifesto inválido: $file"
    format="$(manifest_field "$file" '.format')"
    [[ "$format" == 2 ]] || die "formato de manifesto não suportado: ${format:-vazio}"
    platform="$(manifest_field "$file" '.platform')"
    case "$(uname -m)" in
        x86_64|amd64) expected='linux-x86_64' ;;
        aarch64|arm64) expected='linux-aarch64' ;;
        *) expected="linux-$(uname -m)" ;;
    esac
    [[ "$platform" == "$expected" ]] \
        || die "manifesto é para $platform, esta máquina é $expected"

    # Auro assina os campos nesta ordem, sem o campo `signature`.
    jq -c '{format,version,platform,url,size,sha256,notes,published_at}
           + (if has("delta") then {delta} else {} end)
           + (if has("rollout") then {rollout} else {} end)' \
        "$file" > "$tmp/payload.json"
    manifest_field "$file" '.signature' | base64 -d > "$tmp/signature.bin"
    [[ -s "$tmp/signature.bin" ]] || die "manifesto sem assinatura"

    # SubjectPublicKeyInfo DER de uma chave Ed25519 de 32 bytes.
    printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00' > "$tmp/public.der"
    printf '%s' "$key" | base64 -d >> "$tmp/public.der"
    openssl pkeyutl -verify -pubin -inkey "$tmp/public.der" -rawin \
        -in "$tmp/payload.json" -sigfile "$tmp/signature.bin" >/dev/null 2>&1 \
        || die "assinatura do manifesto não confere com a chave fixada"
}

verify_file() {
    local file="$1" size="$2" sha="$3" label="$4" actual
    [[ -f "$file" ]] || die "$label não encontrado"
    if [[ -n "$size" ]]; then
        actual="$(wc -c < "$file")"
        [[ "$actual" == "$size" ]] \
            || die "$label: tamanho $actual, manifesto espera $size"
    fi
    if [[ -n "$sha" ]]; then
        actual="$(sha256sum "$file" | awk '{print $1}')"
        [[ "$actual" == "$sha" ]] \
            || die "$label: SHA-256 não confere com o manifesto"
    fi
}

verify_appimage() {
    local magic
    magic="$(dd if="$1" bs=1 skip=8 count=2 status=none 2>/dev/null || true)"
    [[ "$magic" == 'AI' ]] || die "$1 não é um AppImage (assinatura AI ausente)"
}

verify_elf() {
    local magic
    magic="$(dd if="$1" bs=1 count=4 status=none 2>/dev/null || true)"
    [[ "$magic" == $'\x7fELF' ]] || die "$1 não é um executável ELF"
}

install_exec() {
    local src="$1" dest="$2"
    mkdir -p "$(dirname "$dest")"
    install -m 0755 "$src" "${dest}.tmp.$$"
    mv -f "${dest}.tmp.$$" "$dest"
}

install_desktop() {
    local apps="$data_dir/applications" icon_name='' safe_name
    mkdir -p "$apps" "$data_dir/icons"
    if [[ -n "$icon" ]]; then
        icon_name="${exec_name}.png"
        install -m 0644 "$tmp/icon" "$data_dir/icons/$icon_name"
    fi
    safe_name="$(printf '%s' "$name" | tr '\n\r\t' '   ' | cut -c1-128)"
    cat > "$apps/${exec_name}.desktop.tmp.$$" <<EOF
[Desktop Entry]
Type=Application
Name=$safe_name
Comment=$safe_name — AppImage
Exec=$exec_name
Icon=${icon_name:-$exec_name}
Terminal=false
Categories=Utility;Development;
StartupNotify=true
EOF
    mv -f "$apps/${exec_name}.desktop.tmp.$$" "$apps/${exec_name}.desktop"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$apps" >/dev/null 2>&1 || true
    fi
}

app="$tmp/appimage"
cli=''

if [[ -n "$manifest" ]]; then
    local_or_url "$manifest" "$tmp/manifest.json"
    verify_manifest "$tmp/manifest.json"
    manifest_url="$(manifest_field "$tmp/manifest.json" '.url')"
    [[ -n "$manifest_url" ]] || die "manifesto sem URL do artefato"
    if [[ -z "$source" ]]; then
        source="$manifest_url"
    elif [[ "$source" == http* && "$source" != "$manifest_url" ]]; then
        die "a URL informada difere da URL assinada no manifesto"
    fi
fi

if [[ -z "$source" ]]; then
    fetch "$API" "$tmp/release.json"
    tag="$(manifest_field "$tmp/release.json" '.tag_name')"
    [[ -n "$tag" ]] || die "a API não devolveu uma release estável"
    manifest_url="$(
        jq -r '.assets[]? | select(.name == "auro-update.json") | .browser_download_url' \
            "$tmp/release.json" | head -n 1
    )"
    cli_manifest_url="$(
        jq -r '.assets[]? | select(.name == "auri-cli.json") | .browser_download_url' \
            "$tmp/release.json" | head -n 1
    )"
    icon_url="$(
        jq -r '.assets[]? | select(.name == "icon.png") | .browser_download_url' \
            "$tmp/release.json" | head -n 1
    )"
    [[ -n "$manifest_url" ]] || die "release $tag sem auro-update.json assinado"
    [[ -n "$cli_manifest_url" ]] || die "release $tag sem auri-cli.json assinado"

    fetch "$manifest_url" "$tmp/manifest.json"
    verify_manifest "$tmp/manifest.json"
    version="$(manifest_field "$tmp/manifest.json" '.version')"
    [[ "${tag#v}" == "$version" ]] \
        || die "release $tag e manifesto $version divergem"

    source="$(manifest_field "$tmp/manifest.json" '.url')"
    [[ -n "$source" ]] || die "manifesto sem URL do AppImage"

    if [[ "$cli" == true ]]; then
        fetch "$cli_manifest_url" "$tmp/cli-manifest.json"
        verify_manifest "$tmp/cli-manifest.json"
        [[ "$(manifest_field "$tmp/cli-manifest.json" '.version')" == "$version" ]] \
            || die "manifesto do CLI não corresponde ao da release"
        fetch "$(manifest_field "$tmp/cli-manifest.json" '.url')" "$tmp/cli"
        verify_file "$tmp/cli" \
            "$(manifest_field "$tmp/cli-manifest.json" '.size')" \
            "$(manifest_field "$tmp/cli-manifest.json" '.sha256')" 'CLI'
        verify_elf "$tmp/cli"
        cli="$tmp/cli"
    fi

    if [[ -n "$icon_url" ]] && curl -fL -o "$tmp/icon" "$icon_url" 2>/dev/null; then
        icon="$tmp/icon"
    else
        warn "ícone não disponível na release; entrada .desktop usará o nome do app"
    fi

    name="${name:-Auri}"
    exec_name="${exec_name:-auri-canvas}"
else
    name="${name:-AppImage}"
    exec_name="${exec_name:-appimage}"
    [[ -n "$sha256" || -n "$manifest" ]] \
        || warn "sem manifesto ou --sha256: integridade do download não verificada"
fi

case "$source" in
    http://*|https://*) fetch "$source" "$app" ;;
    *) local_or_url "$source" "$app" ;;
esac

if [[ -f "$tmp/manifest.json" ]]; then
    verify_file "$app" \
        "$(manifest_field "$tmp/manifest.json" '.size')" \
        "$(manifest_field "$tmp/manifest.json" '.sha256')" 'AppImage'
elif [[ -n "$sha256" ]]; then
    verify_file "$app" '' "$sha256" 'AppImage'
fi
verify_appimage "$app"

install_exec "$app" "$install_dir/$exec_name"
if [[ -n "$cli" ]]; then
    install_exec "$cli" "$install_dir/auri"
fi
if [[ "$desktop" == true ]]; then
    install_desktop
fi

printf 'instalado: %s/%s\n' "$install_dir" "$exec_name"
[[ -n "$cli" ]] && printf 'instalado: %s/auri\n' "$install_dir"
[[ "$desktop" == true ]] && printf 'desktop: %s/applications/%s.desktop\n' "$data_dir" "$exec_name"
