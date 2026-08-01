#!/usr/bin/env bash
set -euo pipefail

repository=""
git_ref=""
output_directory="target/macos-ci-artifacts"

usage() {
  cat <<'EOF'
Usage: .github/scripts/run-macos-package-workflow.sh \
  [--repo OWNER/REPO] \
  [--ref BRANCH_OR_TAG] \
  [--output-dir PATH]

Dispatch the macOS Package GitHub Actions workflow, wait for it to finish,
download the signed and notarized installer, and verify its SHA-256 checksum.
The workflow and credentials must already be present in the GitHub repository.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      [[ $# -ge 2 ]] || { echo "error: --repo requires a value" >&2; exit 2; }
      repository="$2"
      shift 2
      ;;
    --ref)
      [[ $# -ge 2 ]] || { echo "error: --ref requires a value" >&2; exit 2; }
      git_ref="$2"
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || { echo "error: --output-dir requires a value" >&2; exit 2; }
      output_directory="$2"
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

for command_name in gh shasum; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: required command not found: $command_name" >&2
    exit 1
  fi
done

gh auth status >/dev/null
if [[ -z "$repository" ]]; then
  repository="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
fi

gh workflow view macos-pkg.yml --repo "$repository" >/dev/null

dispatch_arguments=(macos-pkg.yml --repo "$repository")
if [[ -n "$git_ref" ]]; then
  dispatch_arguments+=(--ref "$git_ref")
fi

echo "Dispatching macOS Package for $repository${git_ref:+ at $git_ref}"
dispatch_output="$(gh workflow run "${dispatch_arguments[@]}")"
echo "$dispatch_output"

run_url="$(grep -Eo 'https://[^[:space:]]+/actions/runs/[0-9]+' <<<"$dispatch_output" | tail -1 || true)"
if [[ -z "$run_url" ]]; then
  echo "error: GitHub accepted the dispatch but did not return a workflow run URL" >&2
  echo "Inspect the run with: gh run list --repo $repository --workflow macos-pkg.yml" >&2
  exit 1
fi
run_id="${run_url##*/}"
run_output_directory="$output_directory/$run_id"
if [[ -e "$run_output_directory" ]]; then
  echo "error: refusing to overwrite existing workflow output: $run_output_directory" >&2
  exit 1
fi
mkdir -p "$run_output_directory"

set +e
gh run watch "$run_id" --repo "$repository" --exit-status
workflow_exit_code=$?
set -e

if [[ $workflow_exit_code -ne 0 ]]; then
  gh run download "$run_id" \
    --repo "$repository" \
    --name macos-notarization-diagnostics \
    --dir "$run_output_directory" 2>/dev/null || true
  echo "error: macOS Package failed: $run_url" >&2
  if [[ -n "$(find "$run_output_directory" -type f -print -quit)" ]]; then
    echo "Downloaded failure diagnostics to: $run_output_directory" >&2
  fi
  exit "$workflow_exit_code"
fi

gh run download "$run_id" \
  --repo "$repository" \
  --name artifacts-build-macos-pkg \
  --dir "$run_output_directory"

shopt -s nullglob
checksum_files=("$run_output_directory"/*.pkg.sha256)
if [[ ${#checksum_files[@]} -eq 0 ]]; then
  echo "error: downloaded workflow artifact does not contain a .pkg.sha256 file" >&2
  exit 1
fi
for checksum_file in "${checksum_files[@]}"; do
  (
    cd "$(dirname "$checksum_file")"
    shasum -a 256 -c "$(basename "$checksum_file")"
  )
done

echo "Workflow succeeded: $run_url"
echo "Downloaded verified installer to: $run_output_directory"
