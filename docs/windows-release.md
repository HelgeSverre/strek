# Windows installer

Strek's Windows installer is a WiX v3 MSI built by cargo-dist on a native
Windows runner. It is a conventional desktop installer and does not use the
Microsoft Store.

The interactive installer defaults to `%ProgramFiles%\Strek` and lets the user
choose another directory. It also exposes the system `PATH` integration as an
optional feature, creates a Start Menu shortcut, registers Strek in Apps &
Features, supports in-place major upgrades, and removes those resources during
uninstall.

The stable upgrade and component GUIDs live in `apps/gpui/Cargo.toml` and
`apps/gpui/wix/main.wxs`. Do not regenerate them for a later release: changing
them would break Windows Installer's upgrade and component tracking.

## Test without creating a release

Run **Actions → Windows Installer → Run workflow**. The workflow builds the MSI,
verifies its SHA-256 checksum, silently installs it into a custom path containing
spaces, checks the executable and Start Menu shortcut, uninstalls it, and checks
that both were removed. The MSI and checksum are attached to the workflow run.

The equivalent GitHub CLI commands are:

```sh
gh workflow run windows-msi.yml --ref main
gh run watch --exit-status
```

The release plan can be checked locally on any supported cargo-dist host:

```sh
dist plan --output-format=json
```

The checked-in WiX definition is intentionally customized, so do not replace it
with `dist generate --mode msi`. CI checks that it remains valid XML, while the
manual Windows workflow compiles and exercises the real MSI.

Building the MSI itself requires Windows and the WiX v3 toolchain. GitHub's
`windows-2022` runner includes WiX v3.

## Publish a release

The package version controls the MSI version. A matching `vX.Y.Z` tag causes the
existing release workflow to build and attach these Windows artifacts:

- `strek-x86_64-pc-windows-msvc.msi`
- `strek-x86_64-pc-windows-msvc.msi.sha256`
- `strek-x86_64-pc-windows-msvc.zip`
- `strek-x86_64-pc-windows-msvc.zip.sha256`

## Authenticode signing

The MSI is initially unsigned. This does not affect installation functionality,
but public releases should eventually be Authenticode-signed to establish a
publisher identity and build SmartScreen reputation. Signing does not require
Microsoft Store distribution.

Cargo-dist 0.32 supports SSL.com eSigner. After obtaining a production Windows
code-signing credential, add these repository secrets:

- `SSLDOTCOM_USERNAME`
- `SSLDOTCOM_PASSWORD`
- `SSLDOTCOM_TOTP_SECRET`
- `SSLDOTCOM_CREDENTIAL_ID`

Then add `ssldotcom-windows-sign = "prod"` under `[dist]` in
`dist-workspace.toml`. Do not enable it before the credentials are configured,
because the Windows release job will require them.
