---
name: ews-skill
description: Exchange EWS email tools with local cache, scheduled sync, and OpenClaw bridge.
homepage: https://github.com/hexbyte42-bot/ews-skill
metadata: {"clawdbot":{"emoji":"📧","requires":{"bins":["ews_skillctl","ews_skilld"]},"install":[{"id":"install-script","kind":"shell","command":"bash scripts/install.sh","label":"Install ews-skill daemon + bridge"}]}}
---

# EWS Skill

Use `ews_skillctl` (stdio bridge) with `ews_skilld` (daemon) to access Exchange email tools through a local SQLite cache.

Quick start
- `bash scripts/install.sh`
- Edit `/opt/ews-skill/.env` with Exchange credentials
- `sudo systemctl restart ews-skill-sync.service`
- Call `email_health`, then `email_list`

Install notes
- Installer configures systemd daemon user as the invoking user by default.
- Override with `bash scripts/install.sh --run-user <user>`.

Common tasks
- Read: `email_read` with `{"email_id":"<id>"}`
- Search: `email_search` with `{"query":"keyword","limit":20}`
- Mark read/unread: `email_mark_read` with `{"email_id":"<id>","is_read":true}`
- Move: `email_move` with `{"email_id":"<id>","destination_folder":"inbox"}`
- Send: `email_send` with `{"to":"user@example.com","subject":"...","body":"..."}`

Data semantics
- Time: timestamps are stored and returned in UTC.
- Body fields:
  - `body_html`: raw HTML when available
  - `body_text`: plain text (derived from HTML or from `TextBody` fallback)

Delete behavior (current)
- `email_delete` uses Exchange `SoftDelete`.
- Deleted messages do not move to mailbox `Deleted Items`; they go to Exchange recoverable/deletions area.

Notes
- NTLM is required for this deployment profile.
- Only configured folders are synchronized to local DB.
- If Exchange/network is unavailable, cache may lag until next successful sync.

References
- Setup and operations: `README.md`
- Release process: `docs/releasing.md`
- Validation checklist: `docs/openclaw-ops-checklist.md`
