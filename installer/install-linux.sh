#!/usr/bin/env bash
set -euo pipefail

REPO="razectp/game-optimizer"
REQUESTED_VERSION=""
DRY_RUN=0
PACKAGE_ARG=""

usage() {
    cat <<'EOF'
Uso: install-linux.sh [--version <versão>] [--dry-run] [caminho-do-pacote.deb]

Opções:
  --version, -v <versão>  Baixa e instala uma versão específica do GitHub Releases (ex: v0.6.6 ou 0.6.6).
  --dry-run               Exibe o pacote/URL resolvido e os comandos planejados sem alterar o sistema.
  --help, -h              Exibe esta ajuda.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version|-v)
            [[ $# -ge 2 ]] || { echo "Erro: --version requer um argumento." >&2; exit 64; }
            REQUESTED_VERSION="$2"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        -*)
            echo "Opção desconhecida: $1" >&2
            usage >&2
            exit 64
            ;;
        *)
            if [[ -n "$PACKAGE_ARG" ]]; then
                echo "Erro: informe no máximo um caminho de pacote .deb." >&2
                exit 64
            fi
            PACKAGE_ARG="$1"
            shift
            ;;
    esac
done

for tool in dpkg dpkg-deb apt-get; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Erro: '$tool' não encontrado. Este instalador requer Ubuntu, Linux Mint ou Debian." >&2
        exit 1
    fi
done

architecture=$(dpkg --print-architecture)
case "$architecture" in
    amd64|arm64) ;;
    *)
        echo "Arquitetura não suportada: $architecture (suportado: amd64, arm64)." >&2
        exit 1
        ;;
esac

http_get_text() {
    local url="$1"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -H "Accept: application/vnd.github+json" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- --header="Accept: application/vnd.github+json" "$url"
    else
        echo "Erro: 'curl' ou 'wget' é necessário para baixar o pacote do GitHub Releases." >&2
        exit 1
    fi
}

http_download_file() {
    local url="$1"
    local dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fL --progress-bar "$url" -o "$dest"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --show-progress "$url" -O "$dest"
    else
        echo "Erro: 'curl' ou 'wget' é necessário para baixar o pacote do GitHub Releases." >&2
        exit 1
    fi
}

tmp_dir=$(mktemp -d)
trap 'rm -rf -- "$tmp_dir"' EXIT
chmod 0755 "$tmp_dir"

package=""
download_url=""

if [[ -n "$PACKAGE_ARG" ]]; then
    package=$(realpath -m -- "$PACKAGE_ARG")
    if [[ ! -f "$package" ]]; then
        echo "Pacote .deb não encontrado: $package" >&2
        exit 1
    fi
elif [[ -z "$REQUESTED_VERSION" ]]; then
    script_path="${BASH_SOURCE[0]:-}"
    script_dir=""
    if [[ -n "$script_path" && -f "$script_path" ]]; then
        script_dir=$(cd -- "$(dirname -- "$script_path")" && pwd)
    fi

    if [[ -n "$script_dir" ]]; then
        repo_root=$(cd -- "$script_dir/.." && pwd)
        if [[ -f "$repo_root/Cargo.toml" && -f "$repo_root/installer/linux/revision" ]]; then
            version=$(awk -F'"' '/^version[[:space:]]*=/ { print $2; exit }' "$repo_root/Cargo.toml")
            revision=$(tr -d '[:space:]' <"$repo_root/installer/linux/revision")
            candidate="$repo_root/dist/game-optimizer_${version}-${revision}_${architecture}.deb"
            if [[ -f "$candidate" ]]; then
                package="$candidate"
            fi
        fi
    fi

    if [[ -z "$package" ]]; then
        shopt -s nullglob
        candidates=("$PWD"/game-optimizer_*_"${architecture}".deb)
        if [[ -n "$script_dir" && "$script_dir" != "$PWD" ]]; then
            candidates+=("$script_dir"/game-optimizer_*_"${architecture}".deb)
        fi
        shopt -u nullglob
        if (( ${#candidates[@]} > 0 )); then
            package=$(printf '%s\n' "${candidates[@]}" | sort -V | tail -n1)
        fi
    fi
fi

if [[ -z "$package" ]]; then
    if [[ -n "$REQUESTED_VERSION" ]]; then
        tag="$REQUESTED_VERSION"
        [[ "$tag" == v* ]] || tag="v$tag"
        api_url="https://api.github.com/repos/$REPO/releases/tags/$tag"
    else
        api_url="https://api.github.com/repos/$REPO/releases/latest"
    fi

    echo "Consultando release no GitHub: $api_url"
    release_json=$(http_get_text "$api_url")
    download_url=$(
        printf '%s' "$release_json" |
            grep -oE "\"browser_download_url\"[[:space:]]*:[[:space:]]*\"[^\"]*game-optimizer_[^\"]*_${architecture}\\.deb\"" |
            head -n1 |
            sed -E 's/^"browser_download_url"[[:space:]]*:[[:space:]]*"([^"]*)"$/\1/' || true
    )

    if [[ -z "$download_url" ]]; then
        echo "Erro: nenhum pacote .deb para arquitetura '$architecture' foi encontrado no release." >&2
        exit 1
    fi

    if (( DRY_RUN == 1 )); then
        echo "[dry-run] Pacote remoto: $download_url"
        echo "[dry-run] Comandos planejados:"
        echo "  apt-get update -qq"
        echo "  apt-get install --yes <pacote-baixado.deb>"
        exit 0
    fi

    filename=$(basename -- "$download_url")
    package="$tmp_dir/$filename"
    echo "Baixando $filename..."
    http_download_file "$download_url" "$package"
    chmod 0644 "$package"
fi

if [[ ! -f "$package" || $(dpkg-deb --field "$package" Package 2>/dev/null || true) != game-optimizer ]]; then
    echo "Pacote game-optimizer inválido ou não encontrado: $package" >&2
    exit 1
fi

staged_package="$tmp_dir/$(basename -- "$package")"
if [[ "$package" != "$staged_package" ]]; then
    cp -- "$package" "$staged_package"
    chmod 0644 "$staged_package"
fi

if (( DRY_RUN == 1 )); then
    echo "[dry-run] Pacote local: $package"
    echo "[dry-run] Comandos planejados:"
    echo "  apt-get update -qq"
    echo "  apt-get install --yes $staged_package"
    exit 0
fi

apt_cmd=(/usr/bin/apt-get)
if [[ ! -t 0 ]]; then
    apt_cmd=(/usr/bin/env DEBIAN_FRONTEND=noninteractive /usr/bin/apt-get)
fi

run_privileged() {
    if (( EUID == 0 )); then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo -- "$@"
    else
        echo "Erro: execute como root ou instale 'sudo' para instalar o pacote." >&2
        exit 1
    fi
}

echo "Atualizando índice de pacotes (apt-get update)..."
run_privileged "${apt_cmd[@]}" update -qq
echo "Instalando $(basename -- "$staged_package")..."
run_privileged "${apt_cmd[@]}" install --yes "$staged_package"

echo ""
echo "Game Optimizer instalado com sucesso!"
if command -v game_optimizer >/dev/null 2>&1; then
    game_optimizer --version || true
fi
echo "Abra pelo menu de aplicativos ('Game Optimizer') ou execute: game_optimizer"
