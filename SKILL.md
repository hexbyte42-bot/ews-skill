---
name: ews-skill
description: Exchange EWS email tools with local cache, scheduled sync, and OpenClaw CLI integration.
homepage: https://github.com/hexbyte42-bot/ews-skill
metadata: {"clawdbot":{"emoji":"📧","requires":{"bins":["ews_skillctl","ews_skilld"]},"install":[{"id":"install-script","kind":"shell","command":"bash scripts/install.sh --skill-path \"$HOME/.openclaw/workspace/skill/ews-skill\"","label":"Install ews-skill into OpenClaw skill path"}]}}
---

# EWS Skill

`ews_skillctl` is the primary CLI for OpenClaw and operators. It talks to `ews_skilld` over unix socket.

Agent scope and safety rules
- Prioritize `SKILL.md`, `README.md`, `references/*`, `scripts/install.sh`, and `scripts/uninstall.sh`.
- Do not inspect `src/*` unless the user explicitly asks for code-level debugging or implementation changes.
- After installation completes, always ask one routing question: "Is your mailbox Microsoft 365 (O365/Exchange Online) or On-Prem Exchange?"
- Based on the answer, provide only the corresponding `.env` keys to configure and clearly list which values the user must fill.
- Instruct users to edit `.env` manually on their machine; never request, collect, or echo passwords, tokens, client secrets, or other sensitive values in chat.
- `.env` must never be committed; it is git-ignored in this repository.

Protocol support
- `MAIL_PROTOCOL=ews`: on-prem Exchange (EWS SOAP)
- `MAIL_PROTOCOL=graph`: Microsoft 365 (Graph delegated auth)

Quick start
- `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`
- Edit `$HOME/.openclaw/workspace/skill/ews-skill/.env`
- `sudo systemctl restart ews-skill-sync.service`

Post-install agent flow
- Ask: "Is your mailbox O365 (Microsoft 365) or On-Prem Exchange?"
- If user says O365, guide `.env` with:
  - `MAIL_PROTOCOL=graph`
  - `GRAPH_CLIENT_ID`, `GRAPH_TENANT_ID`
  - shared sync keys (`EWS_SYNC_FOLDERS`, `EWS_SYNC_INTERVAL_SECONDS`)
  - Graph-only sync option (optional `GRAPH_SYNC_MAX_PER_FOLDER`)
- If user says On-Prem, guide `.env` with:
  - `MAIL_PROTOCOL=ews`
  - `EWS_EMAIL`, `EWS_USERNAME`, `EWS_PASSWORD`, `EWS_AUTH_MODE`
  - `EWS_AUTODISCOVER` or `EWS_URL`
  - shared sync keys (`EWS_SYNC_FOLDERS`, `EWS_SYNC_INTERVAL_SECONDS`)
  - EWS-only sync option (optional `EWS_SYNC_LOOKBACK_DAYS`)
- Never ask users to paste secret values into chat; use placeholders and tell them to fill secrets directly in `.env`.

Graph delegated login (single-tenant)
- Set env: `MAIL_PROTOCOL=graph`, `GRAPH_CLIENT_ID`, `GRAPH_TENANT_ID`
- Run: `ews_skillctl login`
  - Uses device code flow: command prints a URL and a short code.
  - User opens the URL in any browser, enters the code, and signs in.
  - Works on headless/remote servers; no local browser is required.
- Clear token cache: `ews_skillctl logout`

Configuration model
- Shared:
  - `MAIL_PROTOCOL`
  - `EWS_SYNC_FOLDERS`
  - `EWS_SYNC_INTERVAL_SECONDS`
- EWS-only:
  - `EWS_EMAIL`, `EWS_PASSWORD`, `EWS_AUTH_MODE`
  - `EWS_AUTH_MODE`: `ntlm` (most common), `basic`
  - `EWS_USERNAME` (usually same as `EWS_EMAIL`; set explicitly if login name differs)
  - `EWS_AUTODISCOVER=true` (try first); use `EWS_URL` only if autodiscovery fails or is blocked
  - `EWS_SYNC_LOOKBACK_DAYS`
- Graph-only:
  - `GRAPH_CLIENT_ID`, `GRAPH_TENANT_ID`
  - `GRAPH_SYNC_MAX_PER_FOLDER` (default `200`, max `200`)

Golden path
- `ews_skillctl tools`
- `ews_skillctl health`
- `ews_skillctl list --folder inbox --limit 20`
- During startup, `health` may return `status=syncing` with progress while initial sync runs.
- If health check fails, see Troubleshooting at the end.

CLI usage
- Discover full command usage:
  - `ews_skillctl --help`
  - `ews_skillctl <command> --help`
- Output modes:
  - default: JSON
  - `--human`: concise human-readable output
- Common examples:

```bash
ews_skillctl health
ews_skillctl list --folder inbox --limit 20
ews_skillctl read --id "<email-id>"
ews_skillctl search --sender "alice@company.com" --subject "invoice" --query "QBR" --limit 20
ews_skillctl delete --id "<email-id>"
ews_skillctl delete --id "<email-id>" --skip-trash
```

- Generic/advanced calls:

```bash
ews_skillctl call email_get_unread --arg folder_name=inbox --arg limit=20
ews_skillctl rpc tools.call --params-json '{"name":"email_health","args":{}}'
```

Sync status monitoring

```bash
# During initial sync
ews_skillctl health
# -> {"status":"syncing","progress":"1/3 folders","synced_folders":1,...}

# When ready
ews_skillctl health
# -> {"status":"ready","progress":"3/3 folders","synced_folders":3,...}
```

Output modes

```bash
# Default: full JSON output (AI-first)
ews_skillctl health
ews_skillctl list --folder inbox --limit 20

# Human summaries (manual checks)
ews_skillctl --human health
ews_skillctl --human list --folder inbox --limit 20
```

Behavior notes
- Timestamps are stored and returned in UTC.
- For user-facing time queries, convert UTC to the user's local timezone before answering.
- `email_delete` default moves to `Deleted Items`; `--skip-trash` uses Exchange `SoftDelete`.
- Shared sync behavior:
  - `email_sync_now` runs immediate sync.
  - `email_add_folder` enrolls a folder and syncs immediately.
  - `EWS_SYNC_FOLDERS` defines sync scope.
  - `EWS_SYNC_INTERVAL_SECONDS` controls background polling in both protocols.
- EWS sync behavior:
  - `EWS_SYNC_LOOKBACK_DAYS > 0` (default `7`): rolling day-window sync via `find_items_since`.
  - `EWS_SYNC_LOOKBACK_DAYS = 0`: unlimited-history sync using incremental sync state.
- Graph sync behavior:
  - Uses latest-N sync per folder (no day-window equivalent).
  - `GRAPH_SYNC_MAX_PER_FOLDER` controls cap per folder.
- CLI search applies a default time window if `--date-from/--date-to` are omitted.
  - default: `30` days (`EWS_CLI_SEARCH_DEFAULT_DAYS`)
  - use `--no-date-limit` to disable per query
- `MAIL_PROTOCOL=graph` supports `health/list_server_folders/list_synced_folders/list/read/search/send/move/delete/mark_read`.
- In Graph mode, `EWS_SYNC_INTERVAL_SECONDS=0` disables background polling (manual `email_sync_now` still works).
- In Graph mode, `email_sync_now` syncs local cache for folders configured in `EWS_SYNC_FOLDERS` (latest `GRAPH_SYNC_MAX_PER_FOLDER` per folder, max 200).
- In Graph mode, `email_add_folder` enrolls and immediately syncs that folder.
- `email_list_synced_folders` counts are cache-derived from local `emails` rows (`total_count`, `unread_count`).

Error handling
- CLI output may include a best-effort normalized message prefix like `[E_NOT_FOUND]`.
- Daemon `tools.call` responses include best-effort normalized `code` values.
- Common codes: `OK`, `E_BAD_ARGS`, `E_AUTH`, `E_NOT_FOUND`, `E_SYNC`, `E_UNKNOWN_TOOL`, `E_INTERNAL`.

Upgrade
- Latest: `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`
- Pinned: `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill" --version vX.Y.Z`
- Upgrade keeps existing `<skill-path>/.env` and cache DB.

Uninstall
- `bash scripts/uninstall.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`
- Purge: `bash scripts/uninstall.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill" --purge`

Troubleshooting
1. Restart daemon first:
   - `sudo systemctl restart ews-skill-sync.service`
   - `sudo systemctl status ews-skill-sync.service`
2. Check logs:
   - `sudo journalctl -u ews-skill-sync.service -n 200 --no-pager`
   - `sudo journalctl -u ews-skill-sync.service -f`
3. If `ews_skillctl` returns `No such file or directory (os error 2)`:
   - socket file is missing; service may have failed to start
   - check service status: `systemctl status ews-skill-sync.service`
   - check logs for startup errors
   - expected socket path: `/run/ews-skill/daemon.sock`
   - then retry `ews_skillctl health`
4. Verify socket and permissions:
   - `ls -l /run/ews-skill/daemon.sock`
   - confirm `EWS_SOCKET_PATH` (if set) matches daemon socket path
   - daemon service user should match OpenClaw runtime user
5. Retry checks:
   - `ews_skillctl tools`
   - `ews_skillctl health`
6. Validate required env in `<skill-path>/.env`:
   - EWS: `MAIL_PROTOCOL=ews`, `EWS_EMAIL`, `EWS_USERNAME`, `EWS_PASSWORD`, `EWS_AUTH_MODE`, `EWS_SYNC_FOLDERS`
   - Graph: `MAIL_PROTOCOL=graph`, `GRAPH_CLIENT_ID`, `GRAPH_TENANT_ID`, `EWS_SYNC_FOLDERS`
7. Reinstall only as last option:
   - `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`

References
- Setup and operations: `README.md`
- Validation checklist: `references/openclaw-ops-checklist.md`
