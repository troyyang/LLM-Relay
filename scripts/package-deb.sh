#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

PACKAGE_NAME="llm-relay"
VERSION="${VERSION:-}"
TARGET_SUPPLIED=0
if [[ -n "${TARGET+x}" ]]; then
  TARGET_SUPPLIED=1
fi
TARGET="${TARGET:-x86_64-unknown-linux-musl}"
DEB_ARCH="${DEB_ARCH:-amd64}"
BUILD_DIR="${BUILD_DIR:-${PROJECT_ROOT}/target/package}"
SKIP_BUILD=0
ORIGINAL_ARGS=("$@")

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

run_in_docker() {
  local platform
  local docker_build_dir
  local docker_image="rust:1-bookworm"
  local docker_host_target
  local docker_target="${TARGET}"

  case "${DEB_ARCH}" in
    amd64)
      platform="linux/amd64"
      ;;
    arm64)
      platform="linux/arm64"
      ;;
    *)
      echo "dpkg-deb is unavailable and Docker packaging supports only amd64 or arm64." >&2
      echo "Install dpkg-deb on a Debian-based Linux host to package ${DEB_ARCH}." >&2
      exit 1
      ;;
  esac

  case "${BUILD_DIR}" in
    "${PROJECT_ROOT}"/*)
      docker_build_dir="/work/${BUILD_DIR#"${PROJECT_ROOT}/"}"
      ;;
    *)
      echo "BUILD_DIR must be within the project root when packaging with Docker." >&2
      exit 1
      ;;
  esac

  if ! docker run --rm --platform "${platform}" "${docker_image}" \
    sh -c 'command -v rustup >/dev/null 2>&1'; then
    if [[ "${TARGET_SUPPLIED}" -eq 0 && "${TARGET}" == "x86_64-unknown-linux-musl" && "${DEB_ARCH}" == "amd64" ]]; then
      docker_target="x86_64-unknown-linux-gnu"
      echo "Docker image has no rustup; using its native ${docker_target} target."
    else
      docker_host_target="$(docker run --rm --platform "${platform}" "${docker_image}" \
        rustc -vV | awk '/^host:/ { print $2; exit }')"
      if [[ "${TARGET}" != "${docker_host_target}" ]]; then
        echo "Docker image has no rustup and cannot install requested target ${TARGET}." >&2
        echo "Use a native GNU/Linux target or package on a host with rustup installed." >&2
        exit 1
      fi
    fi
  fi

  docker_package() {
    exec docker run --rm \
      --platform "${platform}" \
      -v "${PROJECT_ROOT}:/work" \
      -w /work \
      -e "VERSION=${VERSION}" \
      -e "TARGET=${docker_target}" \
      -e "DEB_ARCH=${DEB_ARCH}" \
      -e "BUILD_DIR=${docker_build_dir}" \
      "${docker_image}" \
      bash -lc '
        set -e
        export PATH=/usr/local/cargo/bin:$PATH
        apt-get update
        apt-get install -y --no-install-recommends dpkg-dev musl-tools
        if command -v rustup >/dev/null 2>&1; then
          rustup target add "${TARGET:-x86_64-unknown-linux-musl}"
        fi
        exec ./scripts/package-deb.sh "$@"
      ' llm-relay-package-deb "$@"
  }

  echo "dpkg-deb is unavailable; building the Debian package in Docker (${platform})."
  if [[ "${#ORIGINAL_ARGS[@]}" -eq 0 ]]; then
    docker_package
  else
    docker_package "${ORIGINAL_ARGS[@]}"
  fi
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
      TARGET_SUPPLIED=1
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

cd "${PROJECT_ROOT}"

if ! command -v dpkg-deb >/dev/null 2>&1; then
  require_command docker
  run_in_docker
fi

require_command cargo
require_command rustc

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

if ! getent group llm-relay >/dev/null 2>&1; then
  groupadd --system llm-relay
fi

install -d -o root -g llm-relay -m 0750 /etc/llm-relay
chown root:llm-relay /etc/llm-relay/config.yaml
chmod 0640 /etc/llm-relay/config.yaml

/usr/local/bin/llm-relay generate-key --config /etc/llm-relay/config.yaml >/dev/null
chown root:llm-relay /etc/llm-relay/api_key
chmod 0640 /etc/llm-relay/api_key

if [ -n "${SUDO_USER:-}" ] && [ "${SUDO_USER}" != "root" ] && id "${SUDO_USER}" >/dev/null 2>&1; then
  usermod -a -G llm-relay "${SUDO_USER}"
  echo "Added ${SUDO_USER} to the llm-relay group."
  echo "Sign out and back in before running llm-relay show-key without sudo."
else
  echo "To run llm-relay show-key without sudo, add the operator to the llm-relay group:"
  echo "  sudo usermod -aG llm-relay <user>"
fi

if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
  systemctl daemon-reload
  systemctl enable llm-relay.service
  systemctl start llm-relay.service
  echo "llm-relay installed, enabled, and started."
else
  echo "llm-relay installed. systemd is not running; start it with:"
  echo "  sudo systemctl enable --now llm-relay"
fi
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
