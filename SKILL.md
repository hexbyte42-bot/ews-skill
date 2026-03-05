---
name: ews-skill
description: Exchange EWS email tools with local cache, scheduled sync, and OpenClaw bridge.
homepage: https://github.com/hexbyte42-bot/ews-skill
metadata: {"clawdbot":{"emoji":"📧","requires":{"bins":["ews_skillctl","ews_skilld"]},"install":[{"id":"install-script","kind":"shell","command":"bash scripts/install.sh --skill-path \"$HOME/.openclaw/workspace/skill/ews-skill\"","label":"Install ews-skill into OpenClaw skill path"}]}}
---

# EWS Skill

Use `ews_skillctl` (stdio bridge) with `ews_skilld` (daemon) to access Exchange email tools through a local SQLite cache.

Quick start
- `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`
- Edit `$HOME/.openclaw/workspace/skill/ews-skill/.env` with Exchange credentials
- `sudo systemctl restart ews-skill-sync.service`
- Call `email_health`, then `email_list`

Install notes
- `--skill-path` is required and must be absolute.
- Binaries are installed into `<skill-path>/bin`.
- Installer does a fresh install by removing old service/binaries before reinstall.
- Systemd daemon user defaults to the invoking user; override with `--run-user <user>`.
- Installer refuses to run daemon as `root`.

Upgrade
- Latest: `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`
- Pinned: `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill" --version vX.Y.Z`
- Existing `<skill-path>/.env` and cache DB are preserved during upgrade.
- Verify after upgrade: `sudo systemctl status ews-skill-sync.service`, then `tools.list` and `health.get`.
- Rollback: reinstall previous tag with `--version <previous-tag>`.

Uninstall
- `bash scripts/uninstall.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`
- Optional purge (also remove `.env` + cache DB): `bash scripts/uninstall.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill" --purge`

Common tasks
- Read: `email_read` with `{"email_id":"<id>"}`
- Search: `email_search` with `{"query":"keyword","limit":20}`
- Mark read/unread: `email_mark_read` with `{"email_id":"<id>","is_read":true}`
- Move: `email_move` with `{"email_id":"<id>","destination_folder":"inbox"}`
- Send: `email_send` with `{"to":"user@example.com","subject":"...","body":"..."}`

Data semantics
- Time: timestamps are stored and returned in UTC.
- Sync window: server-side sync is limited to `EWS_SYNC_LOOKBACK_DAYS` for all synced folders (default `7`, set `0` for unlimited).
- Body fields:
  - `body_html`: raw HTML when available
  - `body_text`: plain text (derived from HTML or from `TextBody` fallback)

Delete behavior (current)
- Default: `email_delete` moves messages to `Deleted Items` (Outlook-style behavior).
- Optional: set `skip_trash=true` to bypass `Deleted Items` and use Exchange `SoftDelete`.

Notes
- NTLM is required for this deployment profile.
- Only configured folders are synchronized to local DB.
- If Exchange/network is unavailable, cache may lag until next successful sync.

References
- Setup and operations: `README.md`
- Release process: `docs/releasing.md`
- Validation checklist: `docs/openclaw-ops-checklist.md`
