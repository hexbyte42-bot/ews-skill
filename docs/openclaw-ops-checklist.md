# OpenClaw Operations Checklist

Use this checklist to validate production readiness for OpenClaw + `ews-skill`.

## 1) Pre-deploy

- `ews_skilld` and `ews_skillctl` installed at `<skill-path>/bin/`
- `<skill-path>/.env` exists with required values:
  - `EWS_EMAIL`
  - `EWS_PASSWORD`
  - `EWS_AUTH_MODE=ntlm`
  - `EWS_SYNC_FOLDERS=inbox,sentitems`
  - `EWS_SYNC_LOOKBACK_DAYS=7` (or your configured window)
- NTLM capability check passes:

```bash
<skill-path>/bin/ews_skilld --check-ntlm
```

Expected: `NTLM_SUPPORTED=true`

## 2) Daemon health (systemd)

- Service is active:

```bash
systemctl is-active ews-skill-sync.service
```

Expected: `active`

- Socket exists:

```bash
ls -l /run/ews-skill/daemon.sock
```

- Logs show successful startup and sync loop:

```bash
journalctl -u ews-skill-sync.service -n 100 --no-pager
```

## 3) OpenClaw transport validation

- OpenClaw command points to `<skill-path>/bin/ews_skillctl`
- OpenClaw env includes `EWS_SOCKET_PATH=/run/ews-skill/daemon.sock`

JSON-RPC sanity checks:

1. `tools.list`
2. `health.get` with:
   - `result.success=true`
   - `result.data.auth_ok=true`
   - `result.data.inbox_found=true`

## 4) Read-path functional checks

- `email_list_folders` returns `inbox` and `sentitems`
- `email_list` (`folder_name=inbox`) returns expected messages
- `email_read` works for an ID from `email_list`
- `email_mark_read` true/false succeeds
- `email_search` returns expected matches

## 5) Write-path functional checks

- `email_send` succeeds
- Sent copy appears in `sentitems` after `email_sync_now`
- `email_move` succeeds (for example `sentitems -> inbox`)
- `email_delete` default moves disposable test message to `Deleted Items`
- `email_delete` with `skip_trash=true` bypasses `Deleted Items`

## 6) Resilience checks

- With temporary Exchange/network interruption, cache reads still work
- After restore, `email_sync_now` succeeds and data catches up
- No cross-mailbox cache bleed after mailbox/account change

## 7) Release artifact checks

- GitHub release workflow passed NTLM gate
- Release package includes:
  - `ews_skilld`
  - `ews_skillctl`
  - `config.toml.example`
  - `stdio-service.example.json`
  - `ews-skill-sync.service`
