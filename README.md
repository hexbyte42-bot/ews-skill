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

1. Create env file (local only):

```bash
cp config.toml.example config.toml
```

2. Export runtime env vars:

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

3. Build:

```bash
cargo build --release
```

4. Smoke test:

```bash
./scripts/smoke_test.sh
```

## Release binaries via GitHub Actions

This repo includes `.github/workflows/release.yml` to build and publish release artifacts.

- Trigger release build by pushing a version tag (for example `v0.1.0`)
- Artifacts published to GitHub Release:
  - `ews-skilld-linux-x86_64.tar.gz`
  - `ews-skilld-linux-x86_64.tar.gz.sha256`

Manual run is also available using **workflow_dispatch** from the Actions tab.

## Automatic background syncing

Background sync starts when an `EwsSkill` instance is alive. For OpenClaw external-process mode,
run the stdio JSON-RPC daemon as the long-lived process.

### Option A: run manually

```bash
cargo run --release --bin ews_skilld
```

### Option B: run as systemd service

1. Install project on server:

```bash
sudo mkdir -p /opt/ews-skill
sudo rsync -av ./ /opt/ews-skill/
cd /opt/ews-skill
cargo build --release --bin ews_skilld
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

Use these library entry points:

- `EwsSkill::from_env()` or `EwsSkill::from_config_file(...)`
- `EwsSkill::get_tools()` to register tools
- `EwsSkill::execute_tool(tool_name, args_json)` to execute calls

For external process mode, launch `ews_skilld` and communicate over stdio JSON-RPC.

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
/opt/ews-skill/target/release/ews_skilld
```

Recommended startup handshake from OpenClaw:

1. `tools.list`
2. `health.get`
3. proceed only if `result.success=true` and `result.data.auth_ok=true`

Example:

```rust
use ews_skill::EwsSkill;
use serde_json::json;

fn main() -> Result<(), String> {
    let skill = EwsSkill::from_env()?;

    let tools = EwsSkill::get_tools();
    println!("registered {} tools", tools.len());

    let health = skill.execute_tool("email_health", json!({}));
    println!("health ok: {}", health.success);

    let list = skill.execute_tool(
        "email_list",
        json!({ "folder_name": "Inbox", "limit": 10 }),
    );
    println!("list ok: {}", list.success);

    Ok(())
}
```

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
