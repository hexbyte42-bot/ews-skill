---
name: ews-skill
description: Exchange EWS email tools with local cache, scheduled sync, and OpenClaw bridge.
homepage: https://github.com/hexbyte42-bot/ews-skill
metadata: {"clawdbot":{"emoji":"📧","requires":{"bins":["ews_skillctl","ews_skilld"]},"install":[{"id":"install-script","kind":"shell","command":"bash scripts/install.sh --skill-path \"$HOME/.openclaw/workspace/skill/ews-skill\" --version v0.1.11","label":"Install ews-skill into OpenClaw skill path"}]}}
---

# EWS Skill

Use `ews_skillctl` (stdio JSON-RPC bridge) with `ews_skilld` (unix socket daemon) to access Exchange email tools.

Quick start
- `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill" --version v0.1.11`
- Edit `$HOME/.openclaw/workspace/skill/ews-skill/.env`
- `sudo systemctl restart ews-skill-sync.service`

Golden path
- `tools.list` -> confirm `email_*` tools are present
- `health.get` -> confirm daemon and Exchange connectivity
- `email_list` -> read recent inbox messages

Transport and startup
- OpenClaw should launch: `EWS_SOCKET_PATH=/run/ews-skill/daemon.sock <skill-path>/bin/ews_skillctl`
- Startup checks:
  1. call `tools.list`
  2. call `health.get`
- If startup checks fail, see the Troubleshooting section at the end.

JSON-RPC examples
- `tools.list`

```json
{"jsonrpc":"2.0","id":1,"method":"tools.list","params":{}}
```

- `email_health`

```json
{"jsonrpc":"2.0","id":2,"method":"tools.call","params":{"name":"email_health","args":{}}}
```

- `email_list_folders`

```json
{"jsonrpc":"2.0","id":3,"method":"tools.call","params":{"name":"email_list_folders","args":{}}}
```

- `email_list`

```json
{"jsonrpc":"2.0","id":4,"method":"tools.call","params":{"name":"email_list","args":{"folder_name":"inbox","limit":20,"unread_only":false}}}
```

Expected response shape (`result.data`):

```json
{"emails":[{"id":"<email-id>","subject":"...","datetime_received":"<UTC timestamp>"}]}
```

- `email_read`

```json
{"jsonrpc":"2.0","id":5,"method":"tools.call","params":{"name":"email_read","args":{"email_id":"<email-id>"}}}
```

Expected response shape (`result.data`):

```json
{"id":"<email-id>","subject":"...","body_text":"...","body_html":"...","datetime_received":"<UTC timestamp>"}
```

- `email_search`

```json
{"jsonrpc":"2.0","id":6,"method":"tools.call","params":{"name":"email_search","args":{"query":"invoice","limit":20}}}
```

- `email_get_unread`

```json
{"jsonrpc":"2.0","id":7,"method":"tools.call","params":{"name":"email_get_unread","args":{"folder_name":"inbox","limit":20}}}
```

- `email_mark_read`

```json
{"jsonrpc":"2.0","id":8,"method":"tools.call","params":{"name":"email_mark_read","args":{"email_id":"<email-id>","is_read":true}}}
```

- `email_send`

```json
{"jsonrpc":"2.0","id":9,"method":"tools.call","params":{"name":"email_send","args":{"to":"user@example.com","subject":"Test","body":"Hello from ews-skill"}}}
```

- `email_move`

```json
{"jsonrpc":"2.0","id":10,"method":"tools.call","params":{"name":"email_move","args":{"email_id":"<email-id>","destination_folder":"deleteditems"}}}
```

- `email_delete` (default: move to Deleted Items)

```json
{"jsonrpc":"2.0","id":11,"method":"tools.call","params":{"name":"email_delete","args":{"email_id":"<email-id>"}}}
```

- `email_delete` (`skip_trash=true`: SoftDelete)

```json
{"jsonrpc":"2.0","id":12,"method":"tools.call","params":{"name":"email_delete","args":{"email_id":"<email-id>","skip_trash":true}}}
```

- `email_sync_now`

```json
{"jsonrpc":"2.0","id":13,"method":"tools.call","params":{"name":"email_sync_now","args":{}}}
```

- `email_add_folder`

```json
{"jsonrpc":"2.0","id":14,"method":"tools.call","params":{"name":"email_add_folder","args":{"folder_name":"deleteditems"}}}
```

Behavior notes
- Timestamps are stored and returned in UTC.
- For user-facing time queries, convert UTC timestamps to the user's local timezone before answering.
- `email_delete` default behavior matches Outlook: move to `Deleted Items`.
- `skip_trash=true` uses Exchange `SoftDelete`.
- Sync is server-side windowed by `EWS_SYNC_LOOKBACK_DAYS` for all synced folders.
  - default: `7`
  - set `0` for unlimited history (can be heavy on large mailboxes)

Upgrade
- Latest: `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`
- Pinned: `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill" --version vX.Y.Z`
- Upgrades preserve existing `<skill-path>/.env` and local cache DB.

Uninstall
- `bash scripts/uninstall.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill"`
- Optional purge: `bash scripts/uninstall.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill" --purge`

Troubleshooting
1. Restart daemon first:
   - `sudo systemctl restart ews-skill-sync.service`
   - `sudo systemctl status ews-skill-sync.service`
2. Check logs:
   - `sudo journalctl -u ews-skill-sync.service -n 200 --no-pager`
   - `sudo journalctl -u ews-skill-sync.service -f`
3. Verify socket and permissions:
   - `ls -l /run/ews-skill/daemon.sock`
   - ensure daemon service user matches OpenClaw runtime user
4. Retry startup checks:
   - `tools.list`
   - `health.get`
5. Validate required env in `<skill-path>/.env`:
   - `EWS_EMAIL`, `EWS_PASSWORD`, `EWS_AUTH_MODE=ntlm`
   - `EWS_SYNC_FOLDERS`
   - `EWS_SYNC_LOOKBACK_DAYS`
6. Reinstall only as last option:
   - `bash scripts/install.sh --skill-path "$HOME/.openclaw/workspace/skill/ews-skill" --version v0.1.11`

References
- Setup and operations: `README.md`
- Release process: `docs/releasing.md`
- Validation checklist: `docs/openclaw-ops-checklist.md`
