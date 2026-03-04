#!/usr/bin/env bash
set -euo pipefail

REPO="hexbyte42-bot/ews-skill"
VERSION="latest"
INSTALL_DIR="/opt/ews-skill"
SERVICE_NAME="ews-skill-sync.service"
NO_SYSTEMD="false"
DRY_RUN="false"

usage() {
  cat <<'EOF'
Usage: scripts/install.sh [options]

Options:
  --version <tag>         Install a specific tag (example: v0.1.7). Default: latest
  --install-dir <path>    Install directory. Default: /opt/ews-skill
  --service-name <name>   Systemd unit name. Default: ews-skill-sync.service
  --no-systemd            Skip systemd install/restart
  --dry-run               Print actions without changing system
  -h, --help              Show this help
EOF
}

run() {
  if [[ "$DRY_RUN" == "true" ]]; then
    printf '[dry-run] %s\n' "$*"
  else
    eval "$@"
  fi
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'Missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --install-dir)
      INSTALL_DIR="${2:-}"
      shift 2
      ;;
    --service-name)
      SERVICE_NAME="${2:-}"
      shift 2
      ;;
    --no-systemd)
      NO_SYSTEMD="true"
      shift
      ;;
    --dry-run)
      DRY_RUN="true"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'Unknown option: %s\n' "$1" >&2
      usage
      exit 2
      ;;
  esac
done

need_cmd curl
need_cmd tar
need_cmd sha256sum
need_cmd install

if [[ "$EUID" -ne 0 ]]; then
  SUDO="sudo"
else
  SUDO=""
fi

if [[ "$VERSION" == "latest" ]]; then
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
else
  BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
fi

ARCHIVE="ews-skilld-linux-x86_64.tar.gz"
CHECKSUM_FILE="${ARCHIVE}.sha256"

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

printf 'Installing ews-skill from %s\n' "$BASE_URL"
run "curl -fsSL \"${BASE_URL}/${ARCHIVE}\" -o \"${TMP_DIR}/${ARCHIVE}\""
run "curl -fsSL \"${BASE_URL}/${CHECKSUM_FILE}\" -o \"${TMP_DIR}/${CHECKSUM_FILE}\""

if [[ "$DRY_RUN" == "false" ]]; then
  (
    cd "$TMP_DIR"
    sha256sum -c "$CHECKSUM_FILE"
  )
fi

run "mkdir -p \"${TMP_DIR}/extract\""
run "tar -xzf \"${TMP_DIR}/${ARCHIVE}\" -C \"${TMP_DIR}/extract\""

for bin in ews_skilld ews_skillctl; do
  if [[ "$DRY_RUN" == "false" && ! -f "${TMP_DIR}/extract/${bin}" ]]; then
    printf 'Archive missing binary: %s\n' "$bin" >&2
    exit 1
  fi
done

run "$SUDO mkdir -p \"${INSTALL_DIR}\""
run "$SUDO install -m 0755 \"${TMP_DIR}/extract/ews_skilld\" \"${INSTALL_DIR}/ews_skilld\""
run "$SUDO install -m 0755 \"${TMP_DIR}/extract/ews_skillctl\" \"${INSTALL_DIR}/ews_skillctl\""

if [[ "$DRY_RUN" == "false" ]]; then
  "$INSTALL_DIR/ews_skilld" --check-ntlm
else
  printf '[dry-run] %s --check-ntlm\n' "$INSTALL_DIR/ews_skilld"
fi

if [[ "$DRY_RUN" == "false" && ! -f "${INSTALL_DIR}/.env" ]]; then
  run "$SUDO install -m 0600 \"scripts/ews-skill.env.example\" \"${INSTALL_DIR}/.env\""
  printf 'Created %s/.env from template. Update credentials before starting service.\n' "$INSTALL_DIR"
fi

if [[ "$NO_SYSTEMD" == "false" ]]; then
  if [[ ! -f "systemd/ews-skill-sync.service" ]]; then
    printf 'Missing systemd unit file at systemd/ews-skill-sync.service\n' >&2
    exit 1
  fi

  run "$SUDO install -m 0644 \"systemd/ews-skill-sync.service\" \"/etc/systemd/system/${SERVICE_NAME}\""
  run "$SUDO systemctl daemon-reload"
  run "$SUDO systemctl enable --now \"${SERVICE_NAME}\""
  run "$SUDO systemctl status --no-pager \"${SERVICE_NAME}\""
fi

printf '\nInstall complete.\n'
printf 'Binaries:\n'
printf '  %s/ews_skilld\n' "$INSTALL_DIR"
printf '  %s/ews_skillctl\n' "$INSTALL_DIR"
printf 'Next:\n'
printf '  1) Edit %s/.env\n' "$INSTALL_DIR"
if [[ "$NO_SYSTEMD" == "false" ]]; then
  printf '  2) sudo systemctl restart %s\n' "$SERVICE_NAME"
  printf '  3) sudo journalctl -u %s -f\n' "$SERVICE_NAME"
else
  printf '  2) Start daemon: %s/ews_skilld --transport unix --socket /run/ews-skill/daemon.sock\n' "$INSTALL_DIR"
fi
