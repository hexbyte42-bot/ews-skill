#!/usr/bin/env bash
set -euo pipefail

REPO="hexbyte42-bot/ews-skill"
VERSION="latest"
SKILL_PATH=""
SERVICE_NAME="ews-skill-sync.service"
RUN_USER=""
NO_SYSTEMD="false"
DRY_RUN="false"
SOCKET_PATH="/run/ews-skill/daemon.sock"

usage() {
  cat <<'EOF'
Usage: scripts/install.sh --skill-path <absolute-path> [options]

Options:
  --skill-path <path>     OpenClaw skill root path (required)
  --version <tag>         Install a specific tag (example: vX.Y.Z). Default: latest
  --service-name <name>   Systemd unit name. Default: ews-skill-sync.service
  --run-user <user>       Run daemon as this user (default: invoking user)
  --socket-path <path>    Daemon unix socket path. Default: /run/ews-skill/daemon.sock
  --no-systemd            Skip systemd install/restart
  --dry-run               Print actions without changing system
  -h, --help              Show this help
EOF
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'Missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

run_cmd() {
  if [[ "$DRY_RUN" == "true" ]]; then
    printf '[dry-run]'
    for arg in "$@"; do
      printf ' %q' "$arg"
    done
    printf '\n'
  else
    "$@"
  fi
}

run_maybe_cmd() {
  if [[ "$DRY_RUN" == "true" ]]; then
    run_cmd "$@"
  else
    "$@" || true
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skill-path)
      SKILL_PATH="${2:-}"
      shift 2
      ;;
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --service-name)
      SERVICE_NAME="${2:-}"
      shift 2
      ;;
    --run-user)
      RUN_USER="${2:-}"
      shift 2
      ;;
    --socket-path)
      SOCKET_PATH="${2:-}"
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

if [[ -z "$SKILL_PATH" ]]; then
  printf 'Missing required option: --skill-path\n' >&2
  usage
  exit 2
fi

if [[ "${SKILL_PATH:0:1}" != "/" ]]; then
  printf '--skill-path must be an absolute path: %s\n' "$SKILL_PATH" >&2
  exit 2
fi

need_cmd curl
need_cmd tar
need_cmd sha256sum
need_cmd install
need_cmd id
need_cmd sed

if [[ "$NO_SYSTEMD" == "false" ]]; then
  need_cmd systemctl
fi

if [[ "$EUID" -ne 0 ]]; then
  SUDO="sudo"
else
  SUDO=""
fi

if [[ -z "$RUN_USER" ]]; then
  if [[ -n "${SUDO_USER:-}" ]]; then
    RUN_USER="$SUDO_USER"
  else
    RUN_USER="$(id -un)"
  fi
fi

if ! id "$RUN_USER" >/dev/null 2>&1; then
  printf 'Run user does not exist: %s\n' "$RUN_USER" >&2
  exit 1
fi

if [[ "$RUN_USER" == "root" ]]; then
  printf 'Refusing to install service as root. Use --run-user <openclaw-user>.\n' >&2
  exit 1
fi

RUN_GROUP="$(id -gn "$RUN_USER")"

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

printf 'Preparing fresh install for skill path: %s\n' "$SKILL_PATH"

if [[ "$NO_SYSTEMD" == "false" ]]; then
  run_maybe_cmd $SUDO systemctl stop "$SERVICE_NAME"
  run_maybe_cmd $SUDO systemctl disable "$SERVICE_NAME"
  run_cmd $SUDO rm -f "/etc/systemd/system/${SERVICE_NAME}"
  run_cmd $SUDO systemctl daemon-reload
fi

run_cmd $SUDO rm -f "/opt/ews-skill/ews_skilld" "/opt/ews-skill/ews_skillctl"
run_cmd $SUDO rm -f "${SKILL_PATH}/bin/ews_skilld" "${SKILL_PATH}/bin/ews_skillctl"

printf 'Downloading binaries from %s\n' "$BASE_URL"
run_cmd curl -fsSL "${BASE_URL}/${ARCHIVE}" -o "${TMP_DIR}/${ARCHIVE}"
run_cmd curl -fsSL "${BASE_URL}/${CHECKSUM_FILE}" -o "${TMP_DIR}/${CHECKSUM_FILE}"

if [[ "$DRY_RUN" == "false" ]]; then
  (
    cd "$TMP_DIR"
    sha256sum -c "$CHECKSUM_FILE"
  )
fi

run_cmd mkdir -p "${TMP_DIR}/extract"
run_cmd tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "${TMP_DIR}/extract"

if [[ "$DRY_RUN" == "false" ]]; then
  for bin in ews_skilld ews_skillctl; do
    if [[ ! -f "${TMP_DIR}/extract/${bin}" ]]; then
      printf 'Archive missing binary: %s\n' "$bin" >&2
      exit 1
    fi
  done
fi

run_cmd $SUDO mkdir -p "${SKILL_PATH}/bin"
run_cmd $SUDO install -m 0755 "${TMP_DIR}/extract/ews_skilld" "${SKILL_PATH}/bin/ews_skilld"
run_cmd $SUDO install -m 0755 "${TMP_DIR}/extract/ews_skillctl" "${SKILL_PATH}/bin/ews_skillctl"
run_cmd $SUDO chown -R "${RUN_USER}:${RUN_GROUP}" "${SKILL_PATH}/bin"

if [[ "$DRY_RUN" == "false" ]]; then
  "${SKILL_PATH}/bin/ews_skilld" --check-ntlm
else
  printf '[dry-run] %q --check-ntlm\n' "${SKILL_PATH}/bin/ews_skilld"
fi

if [[ "$DRY_RUN" == "false" && ! -f "${SKILL_PATH}/.env" ]]; then
  run_cmd $SUDO install -m 0600 "scripts/ews-skill.env.example" "${SKILL_PATH}/.env"
  run_cmd $SUDO chown "${RUN_USER}:${RUN_GROUP}" "${SKILL_PATH}/.env"
  printf 'Created %s/.env from template. Update credentials before starting service.\n' "$SKILL_PATH"
fi

if [[ "$NO_SYSTEMD" == "false" ]]; then
  if [[ ! -f "systemd/ews-skill-sync.service" ]]; then
    printf 'Missing systemd template: systemd/ews-skill-sync.service\n' >&2
    exit 1
  fi

  if [[ "$DRY_RUN" == "false" ]]; then
    sed \
      -e "s|__RUN_USER__|${RUN_USER}|g" \
      -e "s|__RUN_GROUP__|${RUN_GROUP}|g" \
      -e "s|__SKILL_PATH__|${SKILL_PATH}|g" \
      -e "s|__SOCKET_PATH__|${SOCKET_PATH}|g" \
      "systemd/ews-skill-sync.service" > "${TMP_DIR}/${SERVICE_NAME}"
  else
    printf '[dry-run] render systemd unit for %s\n' "$SERVICE_NAME"
  fi

  run_cmd $SUDO install -m 0644 "${TMP_DIR}/${SERVICE_NAME}" "/etc/systemd/system/${SERVICE_NAME}"
  run_cmd $SUDO systemctl daemon-reload
  run_cmd $SUDO systemctl enable --now "$SERVICE_NAME"
  run_cmd $SUDO systemctl show -p User -p Group --no-pager "$SERVICE_NAME"
  run_cmd $SUDO systemctl status --no-pager "$SERVICE_NAME"
fi

printf '\nInstall complete.\n'
printf 'Skill path: %s\n' "$SKILL_PATH"
printf 'Daemon user: %s\n' "$RUN_USER"
printf 'Binaries:\n'
printf '  %s/bin/ews_skilld\n' "$SKILL_PATH"
printf '  %s/bin/ews_skillctl\n' "$SKILL_PATH"
printf 'Next:\n'
printf '  1) Edit %s/.env\n' "$SKILL_PATH"
if [[ "$NO_SYSTEMD" == "false" ]]; then
  printf '  2) sudo systemctl restart %s\n' "$SERVICE_NAME"
  printf '  3) sudo journalctl -u %s -f\n' "$SERVICE_NAME"
else
  printf '  2) Start daemon: %s/bin/ews_skilld --transport unix --socket %s\n' "$SKILL_PATH" "$SOCKET_PATH"
fi
