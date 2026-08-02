# macOS release signing

Strek's `macOS Package` workflow builds one universal application for Apple
Silicon and Intel, signs it with Developer ID, creates a signed installer,
submits the installer to Apple's notary service, staples the returned ticket,
and verifies the result with Gatekeeper.

The workflow can be run manually for a release test. Tagged releases also call
it and attach the resulting `.pkg` and SHA-256 checksum to the GitHub Release.

## Later setup checklist

Use this section when returning to finish the setup. The detailed explanations
and credential inventory follow below.

### 1. Commit and push the automation

The workflow and scripts must exist on GitHub before the final test can run.
Commit and push the macOS packaging changes on the branch that should be
tested.

### 2. Create the missing Installer certificate

Generate its private key and Apple certificate signing request locally:

```sh
.github/scripts/prepare-developer-id-certificate.sh request \
  --type installer \
  --common-name "Strek Installer" \
  --email "YOUR-APPLE-ACCOUNT-EMAIL"
```

Sign in to [Apple Certificates, Identifiers &
Profiles](https://developer.apple.com/account/resources/certificates/add),
select **Developer ID Installer**, upload
`target/apple-signing/strek-installer.certSigningRequest`, and download the
issued `.cer` file.

Convert that download into the password-protected `.p12` required by CI:

```sh
.github/scripts/prepare-developer-id-certificate.sh package \
  --type installer \
  --certificate ~/Downloads/developerID_installer.cer \
  --private-key target/apple-signing/strek-installer.key \
  --output target/apple-signing/strek-installer.p12
```

Record the new `.p12` password in a password manager.

### 3. Prepare the existing Application certificate

In **Keychain Access → login → My Certificates**, export
`Developer ID Application: Liseth Solutions AS (9Z2L5FBZS3)` together with its
private key as a password-protected `strek-application.p12`. Record that
password too.

If its private key is unavailable, use the request/package commands from step
2 with `--type application` to create a replacement through Apple's portal.

### 4. Create the notarization API key

Sign in to [App Store Connect](https://appstoreconnect.apple.com/), open your
profile, generate an **Individual API Key**, record its Key ID, and download
`AuthKey_KEYID.p8`. The private key can only be downloaded once.

At this point, the three required files are:

- `strek-application.p12`
- `strek-installer.p12`
- `AuthKey_KEYID.p8`

### 5. Validate and upload the credentials

Authenticate the GitHub CLI if needed:

```sh
gh auth login
```

First validate without changing GitHub:

```sh
.github/scripts/configure-macos-signing.sh \
  --application-p12 /path/to/strek-application.p12 \
  --installer-p12 target/apple-signing/strek-installer.p12 \
  --notary-key /path/to/AuthKey_KEYID.p8 \
  --dry-run
```

If validation succeeds, repeat the command without `--dry-run`. The helper
uploads the secrets and derives all repository variables automatically.

### 6. Add the Homebrew tap token

Create a fine-grained personal access token with access only to
`HelgeSverre/homebrew-tap` and grant it **Contents: Read and write**. In the
`HelgeSverre/strek` repository, add that token as a repository Actions secret
named `HOMEBREW_TAP_TOKEN` under **Settings → Secrets and variables → Actions**.

### 7. Run the live CI test

```sh
.github/scripts/run-macos-package-workflow.sh
```

The command waits for GitHub Actions, downloads the signed and notarized `.pkg`
under `target/macos-ci-artifacts/RUN_ID/`, and verifies its checksum. Install
that package on a Mac and launch Strek normally as the final acceptance test.

After the test succeeds, back up both `.p12` files and the `.p8` file securely,
then remove the unencrypted `target/apple-signing/*.key` files.

## One-time Apple setup

You need three private signing assets from the same Apple developer team:

1. A **Developer ID Application** certificate and private key. This signs
   `Strek.app`.
2. A **Developer ID Installer** certificate and private key. This signs the
   `.pkg` installer.
3. An **App Store Connect API key** in `.p8` format. This authenticates
   `notarytool` without storing an Apple Account password in GitHub.

Apple treats the application and installer certificates as distinct types.
Create them under **Certificates, Identifiers & Profiles → Certificates → + →
Developer ID**. Apple currently requires the Account Holder role to create
Developer ID certificates. See [Apple's Developer ID certificate
instructions](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/).

Apple does not expose Developer ID certificate creation through the App Store
Connect API, so approving each certificate in the Apple portal is unavoidable.
The repository can create the private key and CSR beforehand and package the
downloaded certificate afterward, avoiding Keychain Access for a new
certificate.

The missing Installer certificate can be prepared with:

```sh
.github/scripts/prepare-developer-id-certificate.sh request \
  --type installer \
  --common-name "Strek Installer" \
  --email "YOUR-APPLE-ACCOUNT-EMAIL"
```

This writes a private key and `strek-installer.certSigningRequest` under the
gitignored `target/apple-signing/` directory. In Apple's developer portal:

1. Select **Developer ID Installer**.
2. Upload `target/apple-signing/strek-installer.certSigningRequest`.
3. Download the issued `.cer` file.

Turn the downloaded certificate and its matching private key into CI's `.p12`
file:

```sh
.github/scripts/prepare-developer-id-certificate.sh package \
  --type installer \
  --certificate ~/Downloads/developerID_installer.cer \
  --private-key target/apple-signing/strek-installer.key \
  --output target/apple-signing/strek-installer.p12
```

The script requests a new `.p12` password without echoing it and validates the
certificate type, Apple team, validity, private key, and public-key match.

For the existing Developer ID Application identity, install and export it from
**Keychain Access → login → My Certificates** as a password-protected
`strek-application.p12`, making sure the exported identity includes its private
key. If that private key is unavailable, the same automated request/package
flow can create a replacement by using `--type application`.

At the end you need:

- `strek-application.p12`
- `strek-installer.p12`

Do not use a public certificate by itself. CI cannot sign without the private
key contained in each `.p12` file. Although `target/apple-signing/` is ignored
by Git, it is not a backup. The intermediate `.key` file is unencrypted and
permission-restricted, so move the finished `.p12` into secure storage and
remove the raw key after the `.p12` has been validated and backed up.

For notarization, the simplest credential is an individual App Store Connect
API key:

1. Sign in to [App Store Connect](https://appstoreconnect.apple.com/).
2. Open your profile and generate an **Individual API Key**.
3. Record its Key ID and download `AuthKey_KEYID.p8`. Apple allows the private
   key to be downloaded only once.

An Account Holder or Admin can use a team key from **Users and Access →
Integrations → Team Keys** instead. A team key additionally needs the Issuer ID
shown on that page. See [Apple's App Store Connect API key
instructions](https://developer.apple.com/help/app-store-connect/get-started/app-store-connect-api).

## Upload the credentials to GitHub

Install and authenticate the GitHub CLI if necessary:

```sh
brew install gh
gh auth login
```

First validate the files, passwords, certificate types, private keys, and team
IDs without changing GitHub:

```sh
.github/scripts/configure-macos-signing.sh \
  --application-p12 /path/to/strek-application.p12 \
  --installer-p12 /path/to/strek-installer.p12 \
  --notary-key /path/to/AuthKey_KEYID.p8 \
  --dry-run
```

The script asks for both `.p12` passwords without echoing them. If validation
succeeds, run the same command without `--dry-run`:

```sh
.github/scripts/configure-macos-signing.sh \
  --application-p12 /path/to/strek-application.p12 \
  --installer-p12 /path/to/strek-installer.p12 \
  --notary-key /path/to/AuthKey_KEYID.p8
```

For a team API key, add `--notary-issuer-id UUID`. If the `.p8` filename does
not contain the Key ID, add `--notary-key-id KEYID`. Use `--repo OWNER/REPO`
when running outside this checkout.

The helper stores these encrypted GitHub Actions secrets:

- `APPLE_APPLICATION_CERTIFICATE_BASE64`
- `APPLE_APPLICATION_CERTIFICATE_PASSWORD`
- `APPLE_INSTALLER_CERTIFICATE_BASE64`
- `APPLE_INSTALLER_CERTIFICATE_PASSWORD`
- `APPLE_NOTARY_KEY_BASE64`

It derives and stores these non-secret repository variables:

- `APPLE_APPLICATION_SIGNING_IDENTITY`
- `APPLE_INSTALLER_SIGNING_IDENTITY`
- `APPLE_TEAM_ID`
- `APPLE_NOTARY_KEY_ID`
- `APPLE_NOTARY_ISSUER_ID` for a team API key only

You can confirm the names, but not secret values, with:

```sh
gh secret list --app actions
gh variable list
```

Keep the original credentials in a secure backup such as an encrypted password
manager. Never commit `.p12` or `.p8` files to the repository.

## Test without creating a release

After the workflow and credential configuration have been pushed to GitHub,
dispatch the workflow, wait for it, download the result, and verify its
checksum with:

```sh
.github/scripts/run-macos-package-workflow.sh
```

Use `--ref BRANCH_OR_TAG` if the workflow should run from a non-default ref and
`--repo OWNER/REPO` when running outside this checkout. Successful installers
are downloaded below `target/macos-ci-artifacts/RUN_ID/`. On failure, available
notarization diagnostics are downloaded there instead.

The equivalent manual flow is:

1. Open **Actions → macOS Package → Run workflow**.
2. Wait for `Build signed universal PKG` to finish.
3. Download the `artifacts-build-macos-pkg` workflow artifact.
4. Install the contained `.pkg` on a Mac and launch Strek normally.

The workflow imports the certificates into a temporary keychain on an
ephemeral macOS runner and deletes the keychain and `.p8` file in an `always()`
cleanup step.

For a local structure-only build that deliberately skips Developer ID signing
and notarization:

```sh
cargo install cargo-packager --version 0.11.8 --locked
.github/scripts/package-macos-pkg.sh --unsigned
```

The unsigned package is useful only for build verification and must not be
distributed.

## Publish a release

The Cargo package version is the installer version. Update it before tagging,
then commit and push a matching semantic-version tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The existing `Release` workflow builds the architecture-specific archives and
calls the reusable macOS packaging workflow. The release is created only after
the signed and notarized installer job succeeds. It then publishes the Homebrew
formula to `HelgeSverre/homebrew-tap` using `HOMEBREW_TAP_TOKEN`.

Recipients can download the signed installer from the GitHub Release. Publishing
the `.pkg` on another website is a separate distribution decision and does not
require changing the signing process.

## Credential rotation

Run `configure-macos-signing.sh` again to replace GitHub's stored credentials.
Rotate a certificate before it expires, and revoke an API key immediately if
its `.p8` file is exposed. Developer ID Installer certificates must remain
valid when users launch an installer, so release packages should be rebuilt
with a current installer certificate after rotation.
