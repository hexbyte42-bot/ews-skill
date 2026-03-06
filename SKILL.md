---
name: ews-skill
description: Exchange EWS email tools with local cache, scheduled sync, and OpenClaw CLI integration.
homepage: https://github.com/hexbyte42-bot/ews-skill
metadata: {"clawdbot":{"emoji":"📧","requires":{"bins":["ews_skillctl","ews_skilld"]},"install":[{"id":"install-script","kind":"shell","command":"bash scripts/install.sh --skill-path \"$HOME/.openclaw/workspace/skill/ews-skill\"","label":"Install ews-skill into OpenClaw skill path"}]}}
---

# EWS Skill

`ews_skillctl` is the primary CLI for OpenClaw and operators. It talks to `ews_skilld` over unix socket.

Quick start
- `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`
- Edit `$HOME/.openclaw/workspace/skill/ews-skill/.env`
- `sudo systemctl restart ews-skill-sync.service`

Golden path
- `ews_skillctl --json tools`
- `ews_skillctl --json health`
- `ews_skillctl --json list --folder inbox --limit 20`
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
ews_skillctl --json health
ews_skillctl --json list --folder inbox --limit 20
ews_skillctl --json read --id "<email-id>"
ews_skillctl --json search --sender "alice@company.com" --subject "invoice" --query "QBR" --limit 20
ews_skillctl --json delete --id "<email-id>"
ews_skillctl --json delete --id "<email-id>" --skip-trash
```

- Generic/advanced calls:

```bash
ews_skillctl --json call email_get_unread --arg folder_name=inbox --arg limit=20
ews_skillctl rpc tools.call --params-json '{"name":"email_health","args":{}}'
```

Behavior notes
- Timestamps are stored and returned in UTC.
- For user-facing time queries, convert UTC to the user's local timezone before answering.
- `email_delete` default moves to `Deleted Items`; `--skip-trash` uses Exchange `SoftDelete`.
- Sync uses server-side lookback window `EWS_SYNC_LOOKBACK_DAYS` for all synced folders.
  - default: `7`
  - set `0` for unlimited history (heavier on large mailboxes)
- CLI search applies a default time window if `--date-from/--date-to` are omitted.
  - default: `30` days (`EWS_CLI_SEARCH_DEFAULT_DAYS`)
  - use `--no-date-limit` to disable per query

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
   - this usually means unix socket is not ready yet
   - wait for log line `ews_skilld started` (socket ready)
   - then retry `ews_skillctl --json health`
4. Verify socket and permissions:
   - `ls -l /run/ews-skill/daemon.sock`
   - confirm `EWS_SOCKET_PATH` (if set) matches daemon socket path
   - daemon service user should match OpenClaw runtime user
5. Retry checks:
   - `ews_skillctl --json tools`
   - `ews_skillctl --json health`
6. Validate required env in `<skill-path>/.env`:
   - `EWS_EMAIL`, `EWS_PASSWORD`, `EWS_AUTH_MODE=ntlm`
   - `EWS_SYNC_FOLDERS`, `EWS_SYNC_LOOKBACK_DAYS`
7. Reinstall only as last option:
   - `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`

References
- Setup and operations: `README.md`
- Release process: `docs/releasing.md`
- Validation checklist: `docs/openclaw-ops-checklist.md`
