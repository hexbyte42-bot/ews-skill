# Releasing binaries

This project publishes precompiled release artifacts through GitHub Actions using
`.github/workflows/release.yml`.

## Trigger a release

Push a version tag:

```bash
git tag v0.1.1
git push origin v0.1.1
```

Or run manually from GitHub Actions using **workflow_dispatch**.

## Produced artifacts

- `ews-skilld-linux-x86_64.tar.gz`
- `ews-skilld-linux-x86_64.tar.gz.sha256`

Both are attached to the GitHub Release page for the tag.

## Verify release output

After workflow completion, verify:

1. Artifacts are attached to the release.
2. Checksum file exists and matches tarball.
3. NTLM support probe succeeds:

   ```bash
   ./ews_skilld --check-ntlm
   ```

   Expected output: `NTLM_SUPPORTED=true`
4. Extracted package contains:
   - `ews_skilld`
   - `ews_skillctl`
   - `config.toml.example`
   - `stdio-service.example.json`
   - `ews-skill-sync.service`

## Known note

- If a release fails `--check-ntlm`, mark it as not suitable for NTLM environments.
