#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  .github/scripts/prepare-developer-id-certificate.sh request \
    --type application|installer \
    --common-name NAME \
    --email EMAIL \
    [--output-dir PATH]

  .github/scripts/prepare-developer-id-certificate.sh package \
    --type application|installer \
    --certificate PATH \
    --private-key PATH \
    --output PATH

The request command creates a private key and certificate signing request for
Apple's Developer ID portal. After Apple issues the downloaded .cer file, the
package command validates it against the private key and creates the
password-protected .p12 file used by GitHub Actions.

The package password is read from APPLE_CERTIFICATE_PASSWORD or requested with
a hidden confirmation prompt.
EOF
}

if [[ $# -eq 0 ]]; then
  usage >&2
  exit 2
fi
if [[ "$1" == "-h" || "$1" == "--help" ]]; then
  usage
  exit 0
fi

subcommand="$1"
shift
if [[ "$subcommand" != "request" && "$subcommand" != "package" ]]; then
  echo "error: subcommand must be request or package" >&2
  usage >&2
  exit 2
fi

certificate_type=""
common_name=""
email=""
output_directory="target/apple-signing"
certificate_path=""
private_key_path=""
output_path=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --type)
      [[ $# -ge 2 ]] || { echo "error: --type requires a value" >&2; exit 2; }
      certificate_type="$2"
      shift 2
      ;;
    --common-name)
      [[ $# -ge 2 ]] || { echo "error: --common-name requires a value" >&2; exit 2; }
      common_name="$2"
      shift 2
      ;;
    --email)
      [[ $# -ge 2 ]] || { echo "error: --email requires a value" >&2; exit 2; }
      email="$2"
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || { echo "error: --output-dir requires a value" >&2; exit 2; }
      output_directory="$2"
      shift 2
      ;;
    --certificate)
      [[ $# -ge 2 ]] || { echo "error: --certificate requires a value" >&2; exit 2; }
      certificate_path="$2"
      shift 2
      ;;
    --private-key)
      [[ $# -ge 2 ]] || { echo "error: --private-key requires a value" >&2; exit 2; }
      private_key_path="$2"
      shift 2
      ;;
    --output)
      [[ $# -ge 2 ]] || { echo "error: --output requires a value" >&2; exit 2; }
      output_path="$2"
      shift 2
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

case "$certificate_type" in
  application)
    expected_identity_prefix="Developer ID Application:"
    portal_certificate_name="Developer ID Application"
    ;;
  installer)
    expected_identity_prefix="Developer ID Installer:"
    portal_certificate_name="Developer ID Installer"
    ;;
  *)
    echo "error: --type must be application or installer" >&2
    exit 2
    ;;
esac

for command_name in openssl shasum; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: required command not found: $command_name" >&2
    exit 1
  fi
done

umask 077

case "$subcommand" in
  request)
    if [[ -z "$common_name" || -z "$email" ]]; then
      echo "error: request requires --common-name and --email" >&2
      exit 2
    fi
    if [[ "$common_name" == *"/"* || "$common_name" == *"\\"* || "$common_name" == *$'\n'* || "$common_name" == *$'\r'* ]]; then
      printf '%s\n' "error: --common-name must not contain '/', '\\', or a newline" >&2
      exit 2
    fi
    if [[ "$email" == *"\\"* || ! "$email" =~ ^[^/@[:space:]]+@[^/@[:space:]]+\.[^/@[:space:]]+$ ]]; then
      echo "error: --email is not a valid email address" >&2
      exit 2
    fi

    mkdir -p "$output_directory"
    private_key_path="$output_directory/strek-${certificate_type}.key"
    csr_path="$output_directory/strek-${certificate_type}.certSigningRequest"
    if [[ -e "$private_key_path" || -e "$csr_path" ]]; then
      echo "error: refusing to overwrite an existing key or request in $output_directory" >&2
      exit 1
    fi

    openssl req \
      -new \
      -newkey rsa:2048 \
      -nodes \
      -sha256 \
      -quiet \
      -subj "/CN=$common_name/emailAddress=$email" \
      -keyout "$private_key_path" \
      -out "$csr_path"
    openssl req -in "$csr_path" -noout -verify
    chmod 600 "$private_key_path" "$csr_path"

    echo "Created private key: $private_key_path"
    echo "Created Apple CSR:   $csr_path"
    echo "Next: select $portal_certificate_name in Apple's Developer ID portal, upload the CSR, and download the issued .cer file."
    echo "Keep the private key secure; the downloaded certificate cannot be packaged without it."
    ;;

  package)
    for required_path in "$certificate_path" "$private_key_path"; do
      if [[ -z "$required_path" || ! -f "$required_path" ]]; then
        echo "error: certificate asset does not exist: ${required_path:-<missing path>}" >&2
        exit 2
      fi
    done
    if [[ -z "$output_path" ]]; then
      echo "error: package requires --output" >&2
      exit 2
    fi
    if [[ -e "$output_path" ]]; then
      echo "error: refusing to overwrite existing output: $output_path" >&2
      exit 1
    fi

    if [[ -z "${APPLE_CERTIFICATE_PASSWORD:-}" ]]; then
      read -r -s -p "New .p12 password: " APPLE_CERTIFICATE_PASSWORD
      echo
      read -r -s -p "Confirm .p12 password: " certificate_password_confirmation
      echo
      if [[ "$APPLE_CERTIFICATE_PASSWORD" != "$certificate_password_confirmation" ]]; then
        echo "error: .p12 passwords do not match" >&2
        exit 1
      fi
    fi
    if [[ -z "$APPLE_CERTIFICATE_PASSWORD" ]]; then
      echo "error: .p12 password must not be empty" >&2
      exit 1
    fi

    temporary_directory="$(mktemp -d)"
    certificate_pem="$temporary_directory/certificate.pem"
    temporary_p12="$temporary_directory/certificate.p12"
    cleanup_temporary_directory() {
      rm -rf "$temporary_directory"
    }
    trap cleanup_temporary_directory EXIT

    if ! openssl x509 -in "$certificate_path" -out "$certificate_pem" 2>/dev/null; then
      if ! openssl x509 -inform DER -in "$certificate_path" -out "$certificate_pem" 2>/dev/null; then
        echo "error: certificate is not a readable PEM or DER X.509 certificate" >&2
        exit 1
      fi
    fi

    certificate_subject="$(openssl x509 -in "$certificate_pem" -noout -subject -nameopt multiline)"
    certificate_identity="$(awk -F ' = ' '/commonName|CN/ { print $2; exit }' <<<"$certificate_subject")"
    certificate_team_id="$(awk -F ' = ' '/organizationalUnitName|OU/ { print $2; exit }' <<<"$certificate_subject")"
    if [[ "$certificate_identity" != "$expected_identity_prefix"* ]]; then
      echo "error: expected $expected_identity_prefix certificate, found: ${certificate_identity:-unknown}" >&2
      exit 1
    fi
    if [[ -z "$certificate_team_id" ]]; then
      echo "error: issued certificate does not contain an Apple team ID" >&2
      exit 1
    fi
    if ! openssl x509 -in "$certificate_pem" -noout -checkend 0 >/dev/null; then
      echo "error: issued certificate is expired" >&2
      exit 1
    fi
    if ! openssl pkey -in "$private_key_path" -check -noout >/dev/null 2>&1; then
      echo "error: private key is invalid or encrypted; use the key created by the request command" >&2
      exit 1
    fi

    certificate_public_key_hash="$({
      openssl x509 -in "$certificate_pem" -pubkey -noout \
        | openssl pkey -pubin -outform DER 2>/dev/null
    } | shasum -a 256 | awk '{ print $1 }')"
    private_public_key_hash="$(openssl pkey -in "$private_key_path" -pubout -outform DER 2>/dev/null \
      | shasum -a 256 | awk '{ print $1 }')"
    if [[ "$certificate_public_key_hash" != "$private_public_key_hash" ]]; then
      echo "error: downloaded certificate does not match the supplied private key" >&2
      exit 1
    fi

    P12_PASSWORD="$APPLE_CERTIFICATE_PASSWORD" openssl pkcs12 \
      -export \
      -inkey "$private_key_path" \
      -in "$certificate_pem" \
      -name "$certificate_identity" \
      -passout env:P12_PASSWORD \
      -out "$temporary_p12"

    P12_PASSWORD="$APPLE_CERTIFICATE_PASSWORD" openssl pkcs12 \
      -in "$temporary_p12" \
      -clcerts \
      -nokeys \
      -passin env:P12_PASSWORD \
      -noout

    mkdir -p "$(dirname "$output_path")"
    mv "$temporary_p12" "$output_path"
    chmod 600 "$output_path"

    echo "Created Developer ID .p12: $output_path"
    echo "Identity:                 $certificate_identity"
    echo "Apple team ID:            $certificate_team_id"
    ;;

esac
