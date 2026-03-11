# ews-skill

Exchange email skill for OpenClaw with a local SQLite cache, supporting both EWS (on-prem Exchange) and Microsoft Graph (Microsoft 365).

## Features

- Protocol support: `MAIL_PROTOCOL=ews` and `MAIL_PROTOCOL=graph`
- On-prem Exchange via EWS SOAP
- Microsoft 365 via Graph API (delegated OAuth)
- Local cache in SQLite for fast AI reads
- EWS: day-window incremental sync (`EWS_SYNC_LOOKBACK_DAYS`)
- Graph: latest-N per-folder incremental sync (`GRAPH_SYNC_MAX_PER_FOLDER`)
- EWS AutoDiscover support
- EWS auth modes: `basic`, `ntlm`
- OpenClaw-style tool definitions + dispatcher

## Protocol support

### EWS mode (on-prem Exchange)

```bash
MAIL_PROTOCOL=ews
EWS_EMAIL=user@company.com
EWS_PASSWORD=secret
EWS_AUTH_MODE=ntlm
EWS_AUTODISCOVER=true
```

### Graph mode (Microsoft 365)

```bash
MAIL_PROTOCOL=graph
GRAPH_CLIENT_ID=your-client-id
GRAPH_TENANT_ID=your-tenant-id
```

Graph delegated login/logout:

```bash
ews_skillctl login
ews_skillctl logout
```

If `ews_skilld` is already running in Graph mode with tenant/client configured,
`ews_skillctl login` can reuse daemon-side Graph auth config even when local
`GRAPH_CLIENT_ID` / `GRAPH_TENANT_ID` are not exported.

## Quick start

Use the installer with an OpenClaw skill path.

### One-command installer (recommended)

From this repo checkout:

```bash
SKILL_PATH="$HOME/.openclaw/workspace/skill/ews-skill"
bash scripts/install.sh --skill-path "$SKILL_PATH"
```

Useful flags:

```bash
bash scripts/install.sh --skill-path "$SKILL_PATH" --version vX.Y.Z
bash scripts/install.sh --skill-path "$SKILL_PATH" --no-systemd
bash scripts/install.sh --run-user openclaw
bash scripts/install.sh --skill-path "$SKILL_PATH" --dry-run
```

Installer behavior:

- `--skill-path` is required and binaries are installed to `<skill-path>/bin`.
- Installer removes old ews-skill service/binaries first, then performs a fresh install.
- The systemd service runs as the invoking user by default.
- Override explicitly with `--run-user <user>` when needed.
- Installer refuses to install daemon as `root`.

### Upgrade

Upgrade in place (keeps existing `.env` and cache DB):

```bash
bash scripts/install.sh --skill-path "$SKILL_PATH"
```

Upgrade to a pinned release:

```bash
bash scripts/install.sh --skill-path "$SKILL_PATH" --version vX.Y.Z
```

Post-upgrade checks:

```bash
sudo systemctl status ews-skill-sync.service
```

Then run OpenClaw startup probes: `ews_skillctl --json tools` and `ews_skillctl --json health`.

Rollback (reinstall previous release):

```bash
bash scripts/install.sh --skill-path "$SKILL_PATH" --version <previous-tag>
```

### Uninstall

```bash
bash scripts/uninstall.sh --skill-path "$SKILL_PATH"
# also remove .env and cache DB
bash scripts/uninstall.sh --skill-path "$SKILL_PATH" --purge
```

### Option A: build from source (manual)

```bash
cargo build --release --bin ews_skilld --bin ews_skillctl
mkdir -p "$SKILL_PATH/bin"
cp target/release/ews_skilld "$SKILL_PATH/bin/ews_skilld"
cp target/release/ews_skillctl "$SKILL_PATH/bin/ews_skillctl"
```

### Option B: use precompiled release binary (manual)

```bash
curl -L -o ews-skilld-linux-x86_64.tar.gz \
  https://github.com/hexbyte42-bot/ews-skill/releases/latest/download/ews-skilld-linux-x86_64.tar.gz
curl -L -o ews-skilld-linux-x86_64.tar.gz.sha256 \
  https://github.com/hexbyte42-bot/ews-skill/releases/latest/download/ews-skilld-linux-x86_64.tar.gz.sha256
sha256sum -c ews-skilld-linux-x86_64.tar.gz.sha256
mkdir -p "$SKILL_PATH"
tar -xzf ews-skilld-linux-x86_64.tar.gz -C "$SKILL_PATH"
"$SKILL_PATH/bin/ews_skilld" --check-ntlm
```

Binary paths:

```bash
$SKILL_PATH/bin/ews_skilld
$SKILL_PATH/bin/ews_skillctl
```

### Common setup/run steps (same for both options)

1. Export runtime env vars:

```bash
export EWS_LOG_LEVEL='info'          # trace | debug | info | warn | error

# Retry policy for network/server transient failures
export EWS_RETRY_MAX_ATTEMPTS=5
export EWS_RETRY_BASE_MS=500
export EWS_RETRY_MAX_BACKOFF_MS=10000

# Shared sync controls
export EWS_SYNC_FOLDERS='inbox,sentitems'
export EWS_SYNC_INTERVAL_SECONDS=30

# Protocol selection: ews | graph
export MAIL_PROTOCOL='ews'

# EWS-only
export EWS_EMAIL='user@company.com'
export EWS_PASSWORD='***'
export EWS_USERNAME='user@company.com' # optional; usually same as EWS_EMAIL, set explicitly if login name differs
export EWS_AUTH_MODE='ntlm'          # basic | ntlm
export EWS_AUTODISCOVER=true         # or set EWS_URL
# export EWS_URL='https://mail.company.com/EWS/Exchange.asmx'
export EWS_SYNC_LOOKBACK_DAYS=7

# Graph-only
# export MAIL_PROTOCOL='graph'
# export GRAPH_CLIENT_ID='xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx'
# export GRAPH_TENANT_ID='xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx'
# export GRAPH_SYNC_MAX_PER_FOLDER=200  # latest N per synced folder (max 200)
```

2. Run daemon manually (optional):

```bash
$SKILL_PATH/bin/ews_skilld --transport unix --socket /run/ews-skill/daemon.sock
```

3. Optional smoke test (source checkout only):

```bash
./scripts/smoke_test.sh

# Optional write-path checks
SMOKE_DO_WRITE=true ./scripts/smoke_test.sh

# Optional delete behavior check:
# default delete => Deleted Items, skip_trash=true => SoftDelete
SMOKE_DO_WRITE=true SMOKE_TEST_DELETE_MODES=true ./scripts/smoke_test.sh

# Protocol parity matrix (EWS + Graph, cache/server/error contract checks)
./scripts/parity_matrix.sh

# Run EWS parity only (recommended for NTLM validation)
PARITY_PROTOCOL=ews PARITY_REQUIRE_EWS_AUTH=true ./scripts/parity_matrix.sh

# Run Graph parity only
PARITY_PROTOCOL=graph PARITY_REQUIRE_GRAPH_AUTH=true ./scripts/parity_matrix.sh
```

### Use released binary with OpenClaw

OpenClaw should run `<skill-path>/bin/ews_skillctl` as a CLI client and parse `--json` output.
`ews_skillctl` communicates with systemd-managed `ews_skilld` over Unix socket.

For maintainers who publish release binaries, see `docs/releasing.md`.

## Automatic background syncing

Background sync runs in `ews_skilld` (systemd recommended), while OpenClaw executes `ews_skillctl` CLI commands on demand.

### Option A: run manually

```bash
cargo run --release --bin ews_skilld -- --transport unix --socket /run/ews-skill/daemon.sock
```

### Option B: run as systemd service

Systemd setup uses your chosen `<skill-path>` and generated unit values.

1. Prepare files in skill path:

```bash
SKILL_PATH="$HOME/.openclaw/workspace/skill/ews-skill"
bash scripts/install.sh --skill-path "$SKILL_PATH"
```

2. Create `<skill-path>/.env` with credentials:

```bash
EWS_EMAIL=user@company.com
EWS_PASSWORD=***
EWS_USERNAME=DOMAIN\user
EWS_AUTH_MODE=ntlm
EWS_AUTODISCOVER=true
EWS_SYNC_FOLDERS=inbox,sentitems
EWS_SYNC_INTERVAL_SECONDS=30
EWS_SYNC_LOOKBACK_DAYS=7
EWS_LOG_LEVEL=info
EWS_RETRY_MAX_ATTEMPTS=5
EWS_RETRY_BASE_MS=500
EWS_RETRY_MAX_BACKOFF_MS=10000

# Optional: write daemon logs to file (otherwise stderr)
# EWS_DAEMON_LOG_FILE=/var/log/ews_skilld.log
```

`EWS_SYNC_LOOKBACK_DAYS` controls server-side sync window for all synced folders.

- default: `7` (recommended)
- set `0` for unlimited history (may be heavy on large mailboxes)

3. Install and start service (done by installer):

```bash
sudo systemctl restart ews-skill-sync.service
sudo systemctl status ews-skill-sync.service
```

4. Tail logs:

```bash
sudo journalctl -u ews-skill-sync.service -f
```

## OpenClaw integration

Primary integration mode is external process:

- systemd runs `ews_skilld` (Exchange sync + cache) over Unix socket
- OpenClaw runs `ews_skillctl` CLI subcommands with `--json`

For production rollout and validation, use `docs/openclaw-ops-checklist.md`.

Why this is a good fit for OpenClaw:

- Most read operations are served from local cache for lower latency.
- Exchange traffic is reduced to scheduled incremental sync.
- Transient network/server failures are isolated in the daemon with retry/backoff.
- OpenClaw can self-discover commands with `ews_skillctl --help`.

NTLM requirement note:

- For on-prem Exchange with `EWS_AUTH_MODE=ntlm`, always use a release that passes `--check-ntlm`.

### ews_skillctl commands

- Discover full command usage:
  - `ews_skillctl --help`
  - `ews_skillctl <command> --help`
- Output modes:
  - default: JSON (AI-friendly)
  - `--human`: concise human-readable summaries
- Common examples:
  - health: `ews_skillctl health`
  - list inbox: `ews_skillctl list --folder inbox --limit 20`
  - read email: `ews_skillctl read --id "<email-id>"`
  - search combined filters: `ews_skillctl search --sender "alice@company.com" --subject "invoice" --query "QBR" --limit 20`
  - delete default (move to Deleted Items): `ews_skillctl delete --id "<email-id>"`
  - delete soft (`SoftDelete`): `ews_skillctl delete --id "<email-id>" --skip-trash`
- Search default window:
  - last `30` days if `--date-from/--date-to` are not provided
  - override via `EWS_CLI_SEARCH_DEFAULT_DAYS`
  - disable per query with `--no-date-limit`

Daemon logging:

- Default output: `stderr`
- Level control: `EWS_LOG_LEVEL` (or `RUST_LOG`)
- Optional file output: `EWS_DAEMON_LOG_FILE=/path/to/ews_skilld.log`

Socket path:

- daemon default: `/run/ews-skill/daemon.sock`
- client override: `EWS_SOCKET_PATH=/run/ews-skill/daemon.sock`

### OpenClaw launch example

Minimal command (OpenClaw task):

```bash
$SKILL_PATH/bin/ews_skillctl health
```

Recommended startup handshake from OpenClaw:

1. `$SKILL_PATH/bin/ews_skillctl tools`
2. `$SKILL_PATH/bin/ews_skillctl health`
3. proceed only if health `auth_ok=true`

During startup, health may report `status=syncing` with progress while initial sync runs in background.

Troubleshooting `socket not found` (`No such file or directory (os error 2)`):

- socket file is missing; service may have failed to start
- check service: `sudo systemctl status ews-skill-sync.service`
- check logs: `sudo journalctl -u ews-skill-sync.service -n 200 --no-pager`
- socket path should exist at `/run/ews-skill/daemon.sock`
- verify socket path: `ls -l /run/ews-skill/daemon.sock`
- retry: `$SKILL_PATH/bin/ews_skillctl health`

### Optional: embedded Rust API

If you are not using OpenClaw external process mode, the crate still exposes `EwsSkill` APIs for embedded Rust integration.

## Exposed tools

- `email_health`
- `email_list_server_folders`
- `email_list_synced_folders`
- `email_list`
- `email_read`
- `email_search`
- `email_get_unread`
- `email_mark_read`
- `email_send`
- `email_move`
- `email_delete`
- `email_sync_now`
- `email_add_folder`

## Synchronization behavior

### Shared behavior

- `email_sync_now` triggers immediate sync.
- `email_add_folder` enrolls a folder into sync scope and syncs it immediately.
- `EWS_SYNC_FOLDERS` controls which folders are synced.
- `EWS_SYNC_INTERVAL_SECONDS` controls background polling interval in both modes.
  - In Graph mode, `EWS_SYNC_INTERVAL_SECONDS=0` disables background polling.

### EWS-specific sync behavior

- Uses EWS incremental synchronization state.
- `EWS_SYNC_LOOKBACK_DAYS` limits sync window by age (default `7`, set `0` for unlimited history).

### Graph-specific sync behavior

- Uses latest-N sync per folder, not day-window sync.
- `GRAPH_SYNC_MAX_PER_FOLDER` controls the per-folder cap (default `200`, max `200`).

## Folder count behavior

`email_list_synced_folders` returns counts derived from locally cached `emails` rows:

- `total_count`: number of cached emails in the folder
- `unread_count`: number of cached unread emails in the folder

Example:

```json
{
  "folders": [
    {
      "display_name": "inbox",
      "total_count": 5,
      "unread_count": 2
    }
  ]
}
```

## Error handling

`ews_skillctl --json` CLI error wrapper format:

```json
{
  "error": "[E_NOT_FOUND] Email not found",
  "ok": false
}
```

Daemon `tools.call` tool result format (canonical code is `code`):

```json
{
  "success": false,
  "data": null,
  "error": "[E_NOT_FOUND] Email not found",
  "code": "E_NOT_FOUND"
}
```

`[E_*]` prefixes in error messages are best-effort normalization for readability.
When `code` is present, treat `code` as authoritative.

Common error codes:

- `OK`: success
- `E_BAD_ARGS`: invalid arguments
- `E_AUTH`: authentication/login issue
- `E_NOT_FOUND`: missing email/folder/resource
- `E_SYNC`: sync operation failure
- `E_BUSY`: temporary contention/busy state
- `E_UNKNOWN_TOOL`: requested tool name not recognized
- `E_INTERNAL`: unexpected internal error

Transient busy responses may include `retry_after_ms`. Client retry/backoff is built in; when running manually, retry after a short delay.

## Configuration examples

### EWS mode (on-prem Exchange)

Recommended (AutoDiscover):

```bash
MAIL_PROTOCOL=ews
EWS_EMAIL=user@company.com
EWS_PASSWORD=secret
EWS_AUTH_MODE=ntlm
EWS_AUTODISCOVER=true
EWS_SYNC_FOLDERS=inbox,sentitems
EWS_SYNC_INTERVAL_SECONDS=30
EWS_SYNC_LOOKBACK_DAYS=7
```

Alternative (manual URL):

```bash
MAIL_PROTOCOL=ews
EWS_EMAIL=user@company.com
EWS_PASSWORD=secret
EWS_AUTH_MODE=ntlm
EWS_AUTODISCOVER=false
EWS_URL=https://mail.company.com/EWS/Exchange.asmx
EWS_SYNC_FOLDERS=inbox,sentitems
EWS_SYNC_INTERVAL_SECONDS=30
```

### Graph mode (Microsoft 365)

```bash
MAIL_PROTOCOL=graph
GRAPH_CLIENT_ID=your-client-id
GRAPH_TENANT_ID=your-tenant-id
EWS_SYNC_FOLDERS=inbox,sentitems
EWS_SYNC_INTERVAL_SECONDS=30
GRAPH_SYNC_MAX_PER_FOLDER=200
```

Then authenticate:

```bash
ews_skillctl login
```

`email_delete` behavior:

- default: move to `Deleted Items`
- optional `skip_trash=true`: perform Exchange `SoftDelete`

## Read cached email data directly (SQLite)

The cache DB default path is:

- `~/.local/share/ews-skill/ews_cache.db`

Inspect with `sqlite3`:

```bash
sqlite3 ~/.local/share/ews-skill/ews_cache.db
```

Useful queries:

```sql
-- folders currently cached
SELECT id, display_name, unread_count, total_count, synced_at
FROM folders
ORDER BY display_name;

-- latest emails in inbox
SELECT e.id, e.subject, e.sender_email, e.is_read, e.datetime_received
FROM emails e
JOIN folders f ON f.id = e.folder_id
WHERE LOWER(f.display_name) = 'inbox'
ORDER BY e.datetime_received DESC
LIMIT 20;

-- full content for one email
SELECT id, subject, sender_name, sender_email, body_text, datetime_received
FROM emails
WHERE id = '...';

-- sync state per folder
SELECT folder_id, last_sync_at
FROM sync_states
ORDER BY last_sync_at DESC;
```

Recipient lists are JSON strings in `to_recipients` and `cc_recipients`.

## Notes

- Keep secrets out of git.
- `ntlm` mode uses libcurl transport.
- Cache DB defaults to `~/.local/share/ews-skill/ews_cache.db`.
