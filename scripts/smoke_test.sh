#!/usr/bin/env bash
set -euo pipefail

echo "== ews-skill smoke test runner =="

if [[ -z "${EWS_PASSWORD:-}" || -z "${EWS_EMAIL:-}" ]]; then
  echo "Missing required env vars: EWS_PASSWORD, EWS_EMAIL"
  exit 2
fi

if [[ -z "${EWS_USERNAME:-}" ]]; then
  export EWS_USERNAME="$EWS_EMAIL"
  echo "EWS_USERNAME not set, defaulting to EWS_EMAIL"
fi

if [[ -z "${EWS_URL:-}" && -z "${EWS_AUTODISCOVER:-}" ]]; then
  export EWS_AUTODISCOVER=true
  echo "EWS_URL not set, defaulting EWS_AUTODISCOVER=true"
fi

FOLDER="${SMOKE_FOLDER:-inbox}"
LIMIT="${SMOKE_LIMIT:-10}"

echo "Building example..."
cargo build --example smoke_test

echo "Running read-only smoke test..."
cargo run --example smoke_test -- --folder "$FOLDER" --limit "$LIMIT"

if [[ "${SMOKE_DO_WRITE:-false}" == "true" ]]; then
  if [[ -z "${SMOKE_SEND_TO:-}" ]]; then
    export SMOKE_SEND_TO="$EWS_EMAIL"
    echo "SMOKE_SEND_TO not set, defaulting to EWS_EMAIL"
  fi

  echo "Running write smoke test (send email)..."
  ARGS=(--folder "$FOLDER" --limit "$LIMIT" --do-write --send-to "$SMOKE_SEND_TO")

  if [[ "${SMOKE_TEST_DELETE_MODES:-false}" == "true" ]]; then
    echo "Enabling delete mode behavior check (default delete vs skip_trash=true)..."
    ARGS+=(--test-delete-modes)
  fi

  cargo run --example smoke_test -- "${ARGS[@]}"
fi

echo "Smoke test finished."
