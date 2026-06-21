#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(awk -F'"' '/^version = / { print $2; exit }' "$ROOT/Cargo.toml")"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64) ARCH_LABEL="arm64" ;;
  x86_64) ARCH_LABEL="x64" ;;
  *) echo "unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

echo "==> Building FreeCodex launcher (release)..."
(cd "$ROOT" && cargo build -p codex-plus-launcher --release)

echo "==> Packaging FreeCodex DMG for macOS ${ARCH_LABEL}..."
bash "$ROOT/scripts/installer/macos/package-freecodex-dmg.sh" "$VERSION" "$ARCH_LABEL"

DMG="$ROOT/dist/freecodex/FreeCodex-${VERSION}-macos-${ARCH_LABEL}.dmg"
echo
echo "Done."
echo "  DMG: $DMG"
echo
echo "发给朋友："
echo "  - Apple Silicon → FreeCodex-*-macos-arm64.dmg"
echo "  - Intel Mac     → 在本机执行: arch -x86_64 bash scripts/package-freecodex.sh（需安装 x86_64 target）"