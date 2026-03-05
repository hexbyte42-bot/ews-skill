# ews-skill

EWS email skill for OpenClaw with Outlook-style local cache (SQLite), autodiscover, and NTLM/Basic auth support.

## Features

- On-prem Exchange via EWS SOAP
- Local cache in SQLite for fast AI reads
- Incremental sync with `SyncFolderItems`
- Autodiscover support
- Auth modes: `basic`, `ntlm`
- OpenClaw-style tool definitions + dispatcher

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
bash scripts/install.sh --skill-path "$SKILL_PATH" --version v0.1.9
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

Then run OpenClaw startup probes: `tools.list` and `health.get`.

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
export EWS_EMAIL='user@company.com'
export EWS_PASSWORD='***'
export EWS_USERNAME='DOMAIN\user'   # optional, defaults to EWS_EMAIL
export EWS_AUTH_MODE='ntlm'          # basic | ntlm
export EWS_AUTODISCOVER=true         # or set EWS_URL
# export EWS_URL='https://mail.company.com/EWS/Exchange.asmx'
export EWS_LOG_LEVEL='info'          # trace | debug | info | warn | error

# Retry policy for network/server transient failures
export EWS_RETRY_MAX_ATTEMPTS=5
export EWS_RETRY_BASE_MS=500
export EWS_RETRY_MAX_BACKOFF_MS=10000
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
```

### Use released binary with OpenClaw

OpenClaw should run `<skill-path>/bin/ews_skillctl` (stdio bridge) and communicate with the
systemd-managed daemon socket.

For maintainers who publish release binaries, see `docs/releasing.md`.

## Automatic background syncing

Background sync starts when an `EwsSkill` instance is alive. For OpenClaw external-process mode,
run the stdio JSON-RPC daemon as the long-lived process.

### Option A: run manually

```bash
cargo run --release --bin ews_skilld
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
- OpenClaw runs `ews_skillctl` (stdio bridge) and forwards JSON-RPC to daemon socket

For production rollout and validation, use `docs/openclaw-ops-checklist.md`.

Why this is a good fit for OpenClaw:

- Most read operations are served from local cache for lower latency.
- Exchange traffic is reduced to scheduled incremental sync.
- Transient network/server failures are isolated in the daemon with retry/backoff.
- OpenClaw only needs a simple stdio JSON-RPC contract.

NTLM requirement note:

- For on-prem Exchange with `EWS_AUTH_MODE=ntlm`, always use a release that passes `--check-ntlm`.

### Stdio JSON-RPC methods

- `tools.list`
- `tools.call`
- `health.get`

Request example (`tools.list`):

```json
{"jsonrpc":"2.0","id":1,"method":"tools.list","params":{}}
```

Request example (`tools.call`):

```json
{"jsonrpc":"2.0","id":2,"method":"tools.call","params":{"name":"email_list","args":{"folder_name":"inbox","limit":10}}}
```

Response shape:

```json
{"jsonrpc":"2.0","id":2,"result":{"success":true,"data":{},"error":null,"code":"OK"}}
```

Possible result codes:

- `OK`
- `E_BAD_ARGS`
- `E_UNKNOWN_TOOL`
- `E_AUTH`
- `E_NOT_FOUND`
- `E_SYNC`
- `E_INTERNAL`

Daemon logging:

- Default output: `stderr`
- Level control: `EWS_LOG_LEVEL` (or `RUST_LOG`)
- Optional file output: `EWS_DAEMON_LOG_FILE=/path/to/ews_skilld.log`

Socket path:

- daemon default: `/run/ews-skill/daemon.sock`
- bridge override: `EWS_SOCKET_PATH=/run/ews-skill/daemon.sock`

### OpenClaw launch config example

See `openclaw/stdio-service.example.json` for a ready-to-adapt process definition.

Minimal launch command (OpenClaw):

```bash
$SKILL_PATH/bin/ews_skillctl
```

Recommended startup handshake from OpenClaw:

1. `tools.list`
2. `health.get`
3. proceed only if `result.success=true` and `result.data.auth_ok=true`

### Optional: embedded Rust API

If you are not using OpenClaw external process mode, the crate still exposes `EwsSkill` APIs for embedded Rust integration.

## Exposed tools

- `email_health`
- `email_list_folders`
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
