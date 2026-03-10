#!/usr/bin/env bash
set -euo pipefail

echo "== ews-skill parity matrix =="

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required"
  exit 2
fi

SOCKET="${PARITY_SOCKET:-/tmp/ews-skill-parity.sock}"
LOG_FILE="${PARITY_LOG_FILE:-/tmp/ews-skill-parity.log}"
FAILED=0
DAEMON_PID=""
SKIPPED=0
REQUIRE_EWS_AUTH="${PARITY_REQUIRE_EWS_AUTH:-false}"
REQUIRE_GRAPH_AUTH="${PARITY_REQUIRE_GRAPH_AUTH:-false}"
PARITY_PROTOCOL="${PARITY_PROTOCOL:-both}"

cleanup() {
  if [[ -n "${DAEMON_PID}" ]]; then
    kill "${DAEMON_PID}" >/dev/null 2>&1 || true
    DAEMON_PID=""
  fi
  rm -f "${SOCKET}"
}
trap cleanup EXIT

run_success_json() {
  local header="$1"
  local check="$2"
  shift 2

  local out
  local attempt=0
  while true; do
    set +e
    out="$($@ 2>&1)"
    local rc=$?
    set -e
    if [[ ${rc} -eq 0 ]]; then
      break
    fi
    if [[ ${attempt} -lt 3 && "${out}" == *"Resource temporarily unavailable"* ]]; then
      attempt=$((attempt + 1))
      sleep 0.2
      continue
    fi
    echo "[FAIL] ${header}: command failed"
    echo "${out}"
    FAILED=1
    return
  done

  if ! jq -e "${check}" >/dev/null <<<"${out}"; then
    echo "[FAIL] ${header}: unexpected JSON"
    echo "${out}"
    FAILED=1
    return
  fi

  echo "[PASS] ${header}"
}

run_failure_json() {
  local header="$1"
  local check="$2"
  shift 2

  local out
  set +e
  out="$($@ 2>&1)"
  local rc=$?
  set -e

  if [[ ${rc} -eq 0 ]]; then
    echo "[FAIL] ${header}: command unexpectedly succeeded"
    echo "${out}"
    FAILED=1
    return
  fi

  if ! jq -e "${check}" >/dev/null <<<"${out}"; then
    echo "[FAIL] ${header}: failure JSON mismatch"
    echo "${out}"
    FAILED=1
    return
  fi

  echo "[PASS] ${header}"
}

start_daemon() {
  local protocol="$1"
  rm -f "${SOCKET}"

  if [[ "${protocol}" == "ews" ]]; then
    if [[ -z "${EWS_EMAIL:-}" || -z "${EWS_PASSWORD:-}" ]]; then
      echo "Skipping EWS: EWS_EMAIL/EWS_PASSWORD not set"
      return 1
    fi
    if [[ -z "${EWS_USERNAME:-}" ]]; then
      export EWS_USERNAME="${EWS_EMAIL}"
    fi
    if [[ -z "${EWS_URL:-}" && -z "${EWS_AUTODISCOVER:-}" ]]; then
      export EWS_AUTODISCOVER=true
    fi
    if [[ -z "${EWS_AUTH_MODE:-}" ]]; then
      export EWS_AUTH_MODE=ntlm
    fi
  else
    if [[ -z "${GRAPH_CLIENT_ID:-}" && -n "${CLIENT_ID:-}" ]]; then
      export GRAPH_CLIENT_ID="${CLIENT_ID}"
    fi
    if [[ -z "${GRAPH_TENANT_ID:-}" && -n "${TENANT_ID:-}" ]]; then
      export GRAPH_TENANT_ID="${TENANT_ID}"
    fi
    if [[ -z "${GRAPH_CLIENT_ID:-}" || -z "${GRAPH_TENANT_ID:-}" ]]; then
      echo "Skipping Graph: GRAPH_CLIENT_ID/GRAPH_TENANT_ID not set"
      return 1
    fi
  fi

  MAIL_PROTOCOL="${protocol}" target/debug/ews_skilld --transport unix --socket "${SOCKET}" >"${LOG_FILE}" 2>&1 &
  DAEMON_PID=$!
  sleep 2
  return 0
}

should_run_protocol() {
  local protocol="$1"
  case "${PARITY_PROTOCOL}" in
    both) return 0 ;;
    ews) [[ "${protocol}" == "ews" ]] ;;
    graph) [[ "${protocol}" == "graph" ]] ;;
    *)
      echo "Invalid PARITY_PROTOCOL='${PARITY_PROTOCOL}' (expected: ews|graph|both)"
      exit 2
      ;;
  esac
}

run_protocol_suite() {
  local protocol="$1"
  echo ""
  echo "== Protocol: ${protocol} =="

  if ! start_daemon "${protocol}"; then
    return
  fi

  local health_out
  if ! health_out="$(target/debug/ews_skillctl --json --socket "${SOCKET}" health 2>&1)"; then
    echo "[FAIL] ${protocol} health: command failed"
    echo "${health_out}"
    FAILED=1
    kill "${DAEMON_PID}" >/dev/null 2>&1 || true
    DAEMON_PID=""
    return
  fi

  if ! jq -e '.backend == "'"${protocol}"'" and has("auth_ok") and has("progress")' >/dev/null <<<"${health_out}"; then
    echo "[FAIL] ${protocol} health: unexpected JSON"
    echo "${health_out}"
    FAILED=1
    kill "${DAEMON_PID}" >/dev/null 2>&1 || true
    DAEMON_PID=""
    return
  fi
  echo "[PASS] ${protocol} health"

  if ! jq -e '.auth_ok == true' >/dev/null <<<"${health_out}"; then
    if [[ "${protocol}" == "graph" && "${REQUIRE_GRAPH_AUTH}" == "true" ]]; then
      echo "[FAIL] ${protocol}: auth_ok=false with PARITY_REQUIRE_GRAPH_AUTH=true"
      FAILED=1
      kill "${DAEMON_PID}" >/dev/null 2>&1 || true
      DAEMON_PID=""
      return
    elif [[ "${protocol}" == "ews" ]]; then
      echo "[WARN] ${protocol}: auth_ok=false in health; continuing with functional checks"
    else
      echo "[SKIP] ${protocol}: auth_ok=false, skipping protocol checks"
      SKIPPED=1
      kill "${DAEMON_PID}" >/dev/null 2>&1 || true
      DAEMON_PID=""
      return
    fi
  fi

  run_success_json "${protocol} server folders" 'has("folders") and (.folders|type=="array")' \
    target/debug/ews_skillctl --json --socket "${SOCKET}" call email_list_server_folders

  run_success_json "${protocol} synced folders" 'has("folders") and (.folders|type=="array")' \
    target/debug/ews_skillctl --json --socket "${SOCKET}" call email_list_synced_folders

  run_success_json "${protocol} sync-now" 'has("message") and (.message|type=="string")' \
    target/debug/ews_skillctl --json --socket "${SOCKET}" sync-now

  run_success_json "${protocol} list inbox" 'has("emails") and (.emails|type=="array")' \
    target/debug/ews_skillctl --json --socket "${SOCKET}" list --folder inbox --limit 3

  run_success_json "${protocol} unread inbox" 'has("emails") and (.emails|type=="array")' \
    target/debug/ews_skillctl --json --socket "${SOCKET}" call email_get_unread --arg folder_name=inbox --arg limit=3

  run_success_json "${protocol} search inbox" 'has("results") and (.results|type=="array")' \
    target/debug/ews_skillctl --json --socket "${SOCKET}" search --query test --folder inbox --limit 3 --no-date-limit

  run_failure_json "${protocol} error format" '.error | startswith("[E_")' \
    target/debug/ews_skillctl --json --socket "${SOCKET}" call not_a_tool

  kill "${DAEMON_PID}" >/dev/null 2>&1 || true
  DAEMON_PID=""
  rm -f "${SOCKET}"
}

echo "Building binaries..."
cargo build --bin ews_skilld --bin ews_skillctl >/dev/null

if should_run_protocol ews; then
  run_protocol_suite ews
fi

if should_run_protocol graph; then
  run_protocol_suite graph
fi

echo ""
if [[ ${FAILED} -eq 0 ]]; then
  if [[ ${SKIPPED} -eq 1 ]]; then
    echo "Parity matrix passed with skips"
  else
    echo "Parity matrix passed"
  fi
else
  echo "Parity matrix failed"
  exit 1
fi
