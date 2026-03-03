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

There are two ways to obtain the daemon binary. After that, setup/run steps are the same.

### Option A: build from source

```bash
cargo build --release --bin ews_skilld
sudo mkdir -p /opt/ews-skill
sudo cp target/release/ews_skilld /opt/ews-skill/ews_skilld
```

Binary path:

```bash
/opt/ews-skill/ews_skilld
```

### Option B: use precompiled release binary

```bash
curl -L -o ews-skilld-linux-x86_64.tar.gz \
  https://github.com/hexbyte42-bot/ews-skill/releases/latest/download/ews-skilld-linux-x86_64.tar.gz
curl -L -o ews-skilld-linux-x86_64.tar.gz.sha256 \
  https://github.com/hexbyte42-bot/ews-skill/releases/latest/download/ews-skilld-linux-x86_64.tar.gz.sha256
sha256sum -c ews-skilld-linux-x86_64.tar.gz.sha256
mkdir -p /opt/ews-skill
tar -xzf ews-skilld-linux-x86_64.tar.gz -C /opt/ews-skill
```

Binary path:

```bash
/opt/ews-skill/ews_skilld
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

2. Run daemon:

```bash
/opt/ews-skill/ews_skilld
```

3. Optional smoke test (source checkout only):

```bash
./scripts/smoke_test.sh
```

### Use released binary with OpenClaw

OpenClaw can run the prebuilt daemon directly (no Rust toolchain needed). For both source build
and release install, keep the daemon at the same path: `/opt/ews-skill/ews_skilld`.

For maintainers who publish release binaries, see `docs/releasing.md`.

## Automatic background syncing

Background sync starts when an `EwsSkill` instance is alive. For OpenClaw external-process mode,
run the stdio JSON-RPC daemon as the long-lived process.

### Option A: run manually

```bash
cargo run --release --bin ews_skilld
```

### Option B: run as systemd service

Systemd setup is identical for source-build and release-binary installs because both use the same
daemon path: `/opt/ews-skill/ews_skilld`.

1. Install files on server:

```bash
sudo mkdir -p /opt/ews-skill

# Source-build mode
sudo rsync -av ./ /opt/ews-skill/
cd /opt/ews-skill && cargo build --release --bin ews_skilld
cp /opt/ews-skill/target/release/ews_skilld /opt/ews-skill/ews_skilld

# OR release-binary mode
# extract ews_skilld-linux-x86_64.tar.gz into /opt/ews-skill
# (binary is already at /opt/ews-skill/ews_skilld)
```

2. Create `/opt/ews-skill/.env` with credentials:

```bash
EWS_EMAIL=user@company.com
EWS_PASSWORD=***
EWS_USERNAME=DOMAIN\user
EWS_AUTH_MODE=ntlm
EWS_AUTODISCOVER=true
EWS_SYNC_FOLDERS=Inbox,Sent Items
EWS_SYNC_INTERVAL_SECONDS=30
EWS_LOG_LEVEL=info
EWS_RETRY_MAX_ATTEMPTS=5
EWS_RETRY_BASE_MS=500
EWS_RETRY_MAX_BACKOFF_MS=10000

# Optional: write daemon logs to file (otherwise stderr)
# EWS_DAEMON_LOG_FILE=/var/log/ews_skilld.log
```

3. Install and start service:

```bash
sudo cp systemd/ews-skill-sync.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now ews-skill-sync.service
sudo systemctl status ews-skill-sync.service
```

4. Tail logs:

```bash
sudo journalctl -u ews-skill-sync.service -f
```

## OpenClaw integration

Primary integration mode is external process. Launch `ews_skilld` and communicate over stdio JSON-RPC.

Why this is a good fit for OpenClaw:

- Most read operations are served from local cache for lower latency.
- Exchange traffic is reduced to scheduled incremental sync.
- Transient network/server failures are isolated in the daemon with retry/backoff.
- OpenClaw only needs a simple stdio JSON-RPC contract.

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
{"jsonrpc":"2.0","id":2,"method":"tools.call","params":{"name":"email_list","args":{"folder_name":"Inbox","limit":10}}}
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

### OpenClaw launch config example

See `openclaw/stdio-service.example.json` for a ready-to-adapt process definition.

Minimal launch command:

```bash
/opt/ews-skill/ews_skilld
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

-- latest emails in Inbox
SELECT e.id, e.subject, e.sender_email, e.is_read, e.datetime_received
FROM emails e
JOIN folders f ON f.id = e.folder_id
WHERE f.display_name = 'Inbox'
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
