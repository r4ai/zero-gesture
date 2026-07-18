# Releasing Zero Gesture

Zero Gesture is currently distributed as a Windows x64 alpha release. Every
release is published as a GitHub Pre-release with a version such as
`v0.1.0-alpha.0`.

## One-time repository setup

1. Create a fine-grained personal access token that is restricted to this
   repository and has read/write access to **Contents**, **Pull requests**, and
   **Issues**.
2. Store it as the `RELEASE_PLEASE_TOKEN` Actions secret.
3. Allow GitHub Actions to create pull requests in the repository Actions
   settings, protect `main` with the CI checks, and protect the `v*` tag pattern
   so only the release automation account can create release tags.

The personal access token is required because events created with the default
`GITHUB_TOKEN` do not start the tag-based release workflow.

## Normal release flow

1. Use Conventional Commits for changes merged into `main`:
   - `fix:` creates a patch alpha release.
   - `feat:` creates a minor alpha release.
   - A `!` or `BREAKING CHANGE:` creates a minor alpha release while the project
     remains below `1.0.0`.
   - `docs:`, `chore:`, and `ci:` alone do not create releases.
2. Release Please opens or updates a Release PR that changes the application
   version, Cargo lockfile, and `CHANGELOG.md`. Review its contents and merge it
   when ready to release.
3. The merge creates a draft GitHub Pre-release and a matching tag. The tag
   workflow validates the tag, builds the installers, attaches them, and then
   publishes the pre-release.
4. Confirm the published Release contains one NSIS `.exe`, one `.msi`, and
   `SHA256SUMS.txt`. Install and launch each installer on Windows 11 before
   recommending the alpha build to users.

Use `Release-As: 0.1.0-alpha.0` in a commit body only for the initial bootstrap
release or an explicitly reviewed exception.

## Recovery

If the installer build fails, the GitHub Release remains a draft. Fix the
problem and rerun the failed workflow for the same tag; asset upload is
idempotent. Do not publish the draft manually, and do not replace a published
release tag.

## Installing an alpha build

Download assets from the repository's **Releases** page:

- The NSIS `.exe` is intended for most users.
- The `.msi` is suitable for managed Windows installations.

Before installing, compare each downloaded file's SHA-256 hash with
`SHA256SUMS.txt`. In PowerShell:

```powershell
Get-FileHash .\Zero-Gesture-setup.exe -Algorithm SHA256
```

These alpha installers are not code signed yet. Windows SmartScreen may display
a warning; users should install only assets downloaded from this repository's
official GitHub Release.
