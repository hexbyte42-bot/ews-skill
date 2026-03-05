#!/usr/bin/env bash
set -euo pipefail

SKILL_PATH=""
SERVICE_NAME="ews-skill-sync.service"
RUN_USER=""
PURGE="false"
DRY_RUN="false"

usage() {
  cat <<'EOF'
Usage: scripts/uninstall.sh --skill-path <absolute-path> [options]

Options:
  --skill-path <path>     OpenClaw skill root path (required)
  --service-name <name>   Systemd unit name. Default: ews-skill-sync.service
  --run-user <user>       User for purge cache lookup (default: invoking user)
  --purge                 Also remove .env and cache DB for run user
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
    --service-name)
      SERVICE_NAME="${2:-}"
      shift 2
      ;;
    --run-user)
      RUN_USER="${2:-}"
      shift 2
      ;;
    --purge)
      PURGE="true"
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

need_cmd rm
need_cmd systemctl
need_cmd id
need_cmd getent
need_cmd cut

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

RUN_HOME="$(getent passwd "$RUN_USER" | cut -d: -f6)"
if [[ -z "$RUN_HOME" ]]; then
  printf 'Cannot determine home directory for user: %s\n' "$RUN_USER" >&2
  exit 1
fi

printf 'Uninstalling ews-skill from %s\n' "$SKILL_PATH"
run_maybe_cmd $SUDO systemctl stop "$SERVICE_NAME"
run_maybe_cmd $SUDO systemctl disable "$SERVICE_NAME"
run_cmd $SUDO rm -f "/etc/systemd/system/${SERVICE_NAME}"
run_cmd $SUDO systemctl daemon-reload

run_cmd $SUDO rm -f "${SKILL_PATH}/bin/ews_skilld" "${SKILL_PATH}/bin/ews_skillctl"

if [[ "$PURGE" == "true" ]]; then
  run_cmd $SUDO rm -f "${SKILL_PATH}/.env"
  run_cmd $SUDO rm -f "${RUN_HOME}/.local/share/ews-skill/ews_cache.db"
fi

printf '\nUninstall complete.\n'
printf 'Skill path: %s\n' "$SKILL_PATH"
printf 'Service removed: %s\n' "$SERVICE_NAME"
if [[ "$PURGE" == "true" ]]; then
  printf 'Purged: .env and cache DB\n'
else
  printf 'Kept: %s/.env and cache DB (use --purge to remove)\n' "$SKILL_PATH"
fi
