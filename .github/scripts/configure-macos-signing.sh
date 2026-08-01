#!/usr/bin/env bash
set -euo pipefail

application_p12=""
installer_p12=""
notary_key=""
notary_key_id=""
notary_issuer_id=""
repository=""
dry_run=false

usage() {
  cat <<'EOF'
Usage: .github/scripts/configure-macos-signing.sh \
  --application-p12 PATH \
  --installer-p12 PATH \
  --notary-key PATH \
  [--notary-key-id ID] \
  [--notary-issuer-id UUID] \
  [--repo OWNER/REPO] \
  [--dry-run]

Validate Apple signing assets and upload them as GitHub Actions secrets and
variables. Certificate passwords are read from the environment variables
APPLE_APPLICATION_CERTIFICATE_PASSWORD and
APPLE_INSTALLER_CERTIFICATE_PASSWORD, or requested with hidden prompts.

The notary key ID is inferred from an AuthKey_KEYID.p8 filename when omitted.
Omit --notary-issuer-id for an individual App Store Connect API key. Team keys
require the issuer ID shown in App Store Connect.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --application-p12)
      application_p12="${2:-}"
      shift 2
      ;;
    --installer-p12)
      installer_p12="${2:-}"
      shift 2
      ;;
    --notary-key)
      notary_key="${2:-}"
      shift 2
      ;;
    --notary-key-id)
      notary_key_id="${2:-}"
      shift 2
      ;;
    --notary-issuer-id)
      notary_issuer_id="${2:-}"
      shift 2
      ;;
    --repo)
      repository="${2:-}"
      shift 2
      ;;
    --dry-run)
      dry_run=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

for file_path in "$application_p12" "$installer_p12" "$notary_key"; do
  if [[ -z "$file_path" || ! -f "$file_path" ]]; then
    echo "error: signing asset does not exist: ${file_path:-<missing path>}" >&2
    usage >&2
    exit 2
  fi
done

if [[ -z "$notary_key_id" ]]; then
  notary_key_filename="$(basename "$notary_key")"
  if [[ "$notary_key_filename" =~ ^AuthKey_([[:alnum:]]+)\.p8$ ]]; then
    notary_key_id="${BASH_REMATCH[1]}"
  else
    echo "error: --notary-key-id is required unless the key is named AuthKey_KEYID.p8" >&2
    exit 2
  fi
fi

if [[ -z "${APPLE_APPLICATION_CERTIFICATE_PASSWORD:-}" ]]; then
  read -r -s -p "Application certificate .p12 password: " APPLE_APPLICATION_CERTIFICATE_PASSWORD
  echo
fi
if [[ -z "${APPLE_INSTALLER_CERTIFICATE_PASSWORD:-}" ]]; then
  read -r -s -p "Installer certificate .p12 password: " APPLE_INSTALLER_CERTIFICATE_PASSWORD
  echo
fi

certificate_identity() {
  local certificate_path="$1"
  local certificate_password="$2"
  local subject

  subject="$(
    P12_PASSWORD="$certificate_password" openssl pkcs12 \
      -in "$certificate_path" \
      -clcerts \
      -nokeys \
      -passin env:P12_PASSWORD 2>/dev/null \
      | openssl x509 -noout -subject -nameopt multiline
  )"

  awk -F ' = ' '/commonName|CN/ { print $2; exit }' <<<"$subject"
}

certificate_team_id() {
  local certificate_path="$1"
  local certificate_password="$2"
  local subject

  subject="$(
    P12_PASSWORD="$certificate_password" openssl pkcs12 \
      -in "$certificate_path" \
      -clcerts \
      -nokeys \
      -passin env:P12_PASSWORD 2>/dev/null \
      | openssl x509 -noout -subject -nameopt multiline
  )"

  awk -F ' = ' '/organizationalUnitName|OU/ { print $2; exit }' <<<"$subject"
}

certificate_has_private_key() {
  local certificate_path="$1"
  local certificate_password="$2"

  P12_PASSWORD="$certificate_password" openssl pkcs12 \
    -in "$certificate_path" \
    -nocerts \
    -nodes \
    -passin env:P12_PASSWORD 2>/dev/null \
    | grep -Eq -- '-----BEGIN .*PRIVATE KEY-----'
}

application_identity="$(certificate_identity "$application_p12" "$APPLE_APPLICATION_CERTIFICATE_PASSWORD")"
installer_identity="$(certificate_identity "$installer_p12" "$APPLE_INSTALLER_CERTIFICATE_PASSWORD")"
application_team_id="$(certificate_team_id "$application_p12" "$APPLE_APPLICATION_CERTIFICATE_PASSWORD")"
installer_team_id="$(certificate_team_id "$installer_p12" "$APPLE_INSTALLER_CERTIFICATE_PASSWORD")"

if [[ "$application_identity" != "Developer ID Application:"* ]]; then
  echo "error: expected a Developer ID Application certificate, found: ${application_identity:-unknown}" >&2
  exit 1
fi
if [[ "$installer_identity" != "Developer ID Installer:"* ]]; then
  echo "error: expected a Developer ID Installer certificate, found: ${installer_identity:-unknown}" >&2
  exit 1
fi
if ! certificate_has_private_key "$application_p12" "$APPLE_APPLICATION_CERTIFICATE_PASSWORD"; then
  echo "error: application .p12 does not contain its private key" >&2
  exit 1
fi
if ! certificate_has_private_key "$installer_p12" "$APPLE_INSTALLER_CERTIFICATE_PASSWORD"; then
  echo "error: installer .p12 does not contain its private key" >&2
  exit 1
fi
if [[ -z "$application_team_id" || "$application_team_id" != "$installer_team_id" ]]; then
  echo "error: application and installer certificates belong to different Apple teams" >&2
  exit 1
fi
if ! grep -Fq -- "-----BEGIN PRIVATE KEY-----" "$notary_key"; then
  echo "error: notary key is not an App Store Connect .p8 private key" >&2
  exit 1
fi

echo "Application identity: $application_identity"
echo "Installer identity:   $installer_identity"
echo "Apple team ID:        $application_team_id"
echo "Notary key ID:        $notary_key_id"
if [[ -n "$notary_issuer_id" ]]; then
  echo "Notary issuer ID:     $notary_issuer_id"
else
  echo "Notary key type:      individual (no issuer ID)"
fi

if [[ "$dry_run" == true ]]; then
  echo "Dry run complete; no GitHub settings were changed."
  exit 0
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "error: GitHub CLI is not installed" >&2
  exit 1
fi
gh auth status >/dev/null

if [[ -z "$repository" ]]; then
  repository="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
fi

set_secret_from_file() {
  local secret_name="$1"
  local file_path="$2"
  /usr/bin/base64 < "$file_path" | gh secret set "$secret_name" --repo "$repository"
}

set_secret_from_value() {
  local secret_name="$1"
  local secret_value="$2"
  printf '%s' "$secret_value" | gh secret set "$secret_name" --repo "$repository"
}

set_secret_from_file APPLE_APPLICATION_CERTIFICATE_BASE64 "$application_p12"
set_secret_from_value APPLE_APPLICATION_CERTIFICATE_PASSWORD "$APPLE_APPLICATION_CERTIFICATE_PASSWORD"
set_secret_from_file APPLE_INSTALLER_CERTIFICATE_BASE64 "$installer_p12"
set_secret_from_value APPLE_INSTALLER_CERTIFICATE_PASSWORD "$APPLE_INSTALLER_CERTIFICATE_PASSWORD"
set_secret_from_file APPLE_NOTARY_KEY_BASE64 "$notary_key"

gh variable set APPLE_APPLICATION_SIGNING_IDENTITY --repo "$repository" --body "$application_identity"
gh variable set APPLE_INSTALLER_SIGNING_IDENTITY --repo "$repository" --body "$installer_identity"
gh variable set APPLE_TEAM_ID --repo "$repository" --body "$application_team_id"
gh variable set APPLE_NOTARY_KEY_ID --repo "$repository" --body "$notary_key_id"
if [[ -n "$notary_issuer_id" ]]; then
  gh variable set APPLE_NOTARY_ISSUER_ID --repo "$repository" --body "$notary_issuer_id"
else
  gh variable delete APPLE_NOTARY_ISSUER_ID --repo "$repository" 2>/dev/null || true
fi

echo "Configured macOS release signing for $repository."
