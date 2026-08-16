#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

PACKAGE_NAME="llm-relay"
VERSION="${VERSION:-}"
TARGET="${TARGET:-x86_64-unknown-linux-musl}"
DEB_ARCH="${DEB_ARCH:-amd64}"
BUILD_DIR="${BUILD_DIR:-${PROJECT_ROOT}/target/package}"
SKIP_BUILD=0

usage() {
  cat <<USAGE
Usage: scripts/package-deb.sh [options]

Build and package llm-relay as a Debian/Ubuntu .deb package.

Options:
  --version <version>   Package version. Defaults to Cargo.toml package version.
  --target <triple>     Rust target triple. Defaults to x86_64-unknown-linux-musl.
  --arch <arch>         Debian architecture. Defaults to amd64.
  --skip-build          Package an already-built target/<target>/release/llm-relay.
  -h, --help            Show this help.

Environment:
  VERSION               Same as --version.
  TARGET                Same as --target.
  DEB_ARCH              Same as --arch.
  BUILD_DIR             Packaging work/output directory.

Output:
  target/package/llm-relay_<version>_<arch>.deb
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="${2:-}"
      if [[ -z "${VERSION}" ]]; then
        echo "missing value after --version" >&2
        exit 1
      fi
      shift 2
      ;;
    --target)
      TARGET="${2:-}"
      if [[ -z "${TARGET}" ]]; then
        echo "missing value after --target" >&2
        exit 1
      fi
      shift 2
      ;;
    --arch)
      DEB_ARCH="${2:-}"
      if [[ -z "${DEB_ARCH}" ]]; then
        echo "missing value after --arch" >&2
        exit 1
      fi
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "required command not found: $1" >&2
    exit 1
  fi
}

require_command cargo
require_command dpkg-deb
require_command rustc

cd "${PROJECT_ROOT}"

if [[ -z "${VERSION}" ]]; then
  VERSION="$(awk -F '"' '/^version = / { print $2; exit }' Cargo.toml)"
fi

if [[ -z "${VERSION}" ]]; then
  echo "failed to determine package version" >&2
  exit 1
fi

if [[ "${SKIP_BUILD}" -eq 0 ]]; then
  if command -v rustup >/dev/null 2>&1; then
    if ! rustup target list --installed 2>/dev/null | grep -qx "${TARGET}"; then
      echo "Rust target ${TARGET} is not installed." >&2
      echo "Install it with: rustup target add ${TARGET}" >&2
      exit 1
    fi
  else
    HOST_TARGET="$(rustc -vV | awk '/^host:/ { print $2; exit }')"
    if [[ "${TARGET}" != "${HOST_TARGET}" ]]; then
      echo "rustup is not installed and requested target ${TARGET} does not match rustc host ${HOST_TARGET}." >&2
      echo "Install rustup or rerun with: --target ${HOST_TARGET}" >&2
      exit 1
    fi
  fi
  cargo build --release --target "${TARGET}"
fi

BINARY="${PROJECT_ROOT}/target/${TARGET}/release/llm-relay"
if [[ ! -x "${BINARY}" ]]; then
  echo "release binary not found or not executable: ${BINARY}" >&2
  exit 1
fi

PACKAGE_ROOT="${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${DEB_ARCH}"
DEB_PATH="${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${DEB_ARCH}.deb"

rm -rf "${PACKAGE_ROOT}"
install -d \
  "${PACKAGE_ROOT}/DEBIAN" \
  "${PACKAGE_ROOT}/usr/local/bin" \
  "${PACKAGE_ROOT}/etc/llm-relay" \
  "${PACKAGE_ROOT}/etc/systemd/system" \
  "${PACKAGE_ROOT}/usr/share/doc/${PACKAGE_NAME}"

install -m 0755 "${BINARY}" "${PACKAGE_ROOT}/usr/local/bin/llm-relay"
install -m 0644 "${PROJECT_ROOT}/config/config.yaml" "${PACKAGE_ROOT}/etc/llm-relay/config.yaml"
install -m 0644 "${PROJECT_ROOT}/deploy/llm-relay.service" \
  "${PACKAGE_ROOT}/etc/systemd/system/llm-relay.service"
install -m 0644 "${PROJECT_ROOT}/README.md" "${PACKAGE_ROOT}/usr/share/doc/${PACKAGE_NAME}/README.md"
install -m 0644 "${PROJECT_ROOT}/LICENSE" "${PACKAGE_ROOT}/usr/share/doc/${PACKAGE_NAME}/copyright"

cat >"${PACKAGE_ROOT}/DEBIAN/control" <<CONTROL
Package: ${PACKAGE_NAME}
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${DEB_ARCH}
Maintainer: LLM Relay Maintainers <maintainers@example.invalid>
Description: Lightweight HTTP relay for LLM provider APIs
 A stateless Rust HTTP relay that forwards requests to configured LLM
 providers, preserves streaming responses, and supports systemd deployment.
CONTROL

cat >"${PACKAGE_ROOT}/DEBIAN/conffiles" <<CONFFILES
/etc/llm-relay/config.yaml
CONFFILES

cat >"${PACKAGE_ROOT}/DEBIAN/postinst" <<'POSTINST'
#!/usr/bin/env sh
set -e

if command -v systemctl >/dev/null 2>&1; then
  systemctl daemon-reload || true
fi

echo "llm-relay installed."
echo "Generate or view the relay API key with:"
echo "  sudo llm-relay generate-key --config /etc/llm-relay/config.yaml"
echo "  sudo llm-relay show-key --config /etc/llm-relay/config.yaml"
echo "Start the service with:"
echo "  sudo systemctl enable --now llm-relay"
POSTINST

cat >"${PACKAGE_ROOT}/DEBIAN/postrm" <<'POSTRM'
#!/usr/bin/env sh
set -e

if command -v systemctl >/dev/null 2>&1; then
  systemctl daemon-reload || true
fi
POSTRM

chmod 0755 "${PACKAGE_ROOT}/DEBIAN/postinst" "${PACKAGE_ROOT}/DEBIAN/postrm"

dpkg-deb --build --root-owner-group "${PACKAGE_ROOT}" "${DEB_PATH}"

echo "Package built: ${DEB_PATH}"
