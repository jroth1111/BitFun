#!/usr/bin/env bash
set -u
set -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SMOKE_ROOT="${REPO_ROOT}/target/cowork-smoke"
LOG_FILE="${SMOKE_ROOT}/cowork-smoke.log"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${SMOKE_ROOT}/cargo-target}"
export TMPDIR="${TMPDIR:-${SMOKE_ROOT}/tmp}"

mkdir -p "${SMOKE_ROOT}" "${CARGO_TARGET_DIR}" "${TMPDIR}"
: > "${LOG_FILE}"

cd "${REPO_ROOT}"

timestamp() {
  date "+%Y-%m-%dT%H:%M:%S%z"
}

format_command() {
  local formatted=""
  local arg

  for arg in "$@"; do
    if [[ -n "${formatted}" ]]; then
      formatted+=" "
    fi
    formatted+="$(printf "%q" "${arg}")"
  done

  printf "%s" "${formatted}"
}

fail_step() {
  local step_name="$1"
  local command="$2"
  local status="$3"
  local reason="$4"

  {
    printf "[%s] FAILED: %s\n" "$(timestamp)" "${reason}"
    printf "Exit code: %s\n" "${status}"
  } >> "${LOG_FILE}"

  printf "cowork smoke failed\n" >&2
  printf "Step: %s\n" "${step_name}" >&2
  printf "Command: %s\n" "${command}" >&2
  printf "Exit code: %s\n" "${status}" >&2
  printf "Log: %s\n" "${LOG_FILE}" >&2
  printf "\nLast log lines:\n" >&2
  tail -n 40 "${LOG_FILE}" >&2
  exit "${status}"
}

run_step() {
  local step_name="$1"
  shift
  local command
  local status

  command="$(format_command "$@")"
  {
    printf "\n[%s] STEP: %s\n" "$(timestamp)" "${step_name}"
    printf "Command: %s\n" "${command}"
  } >> "${LOG_FILE}"

  printf "Running: %s\n" "${step_name}"

  if "$@" >> "${LOG_FILE}" 2>&1; then
    printf "[%s] PASS: %s\n" "$(timestamp)" "${step_name}" >> "${LOG_FILE}"
  else
    status=$?
    fail_step "${step_name}" "${command}" "${status}" "command failed"
  fi
}

run_negative_smoke() {
  local step_name="$1"
  local expected_code="$2"
  local expected_protocol_code="$3"
  shift 3
  local command
  local output_file
  local status

  command="$(format_command "$@")"
  output_file="${TMPDIR}/${step_name//[^A-Za-z0-9_]/_}.out"
  {
    printf "\n[%s] STEP: %s\n" "$(timestamp)" "${step_name}"
    printf "Command: %s\n" "${command}"
    printf "Expected: nonzero exit and output containing %s and %s\n" \
      "${expected_code}" "${expected_protocol_code}"
  } >> "${LOG_FILE}"

  printf "Running: %s\n" "${step_name}"

  if "$@" > "${output_file}" 2>&1; then
    status=0
    {
      printf "Output:\n"
      cat "${output_file}"
    } >> "${LOG_FILE}"
    fail_step "${step_name}" "${command}" 1 "negative smoke unexpectedly succeeded"
  else
    status=$?
  fi

  {
    printf "Observed exit code: %s\n" "${status}"
    printf "Output:\n"
    cat "${output_file}"
  } >> "${LOG_FILE}"

  if ! grep -q "${expected_code}" "${output_file}"; then
    fail_step "${step_name}" "${command}" 1 "negative smoke output missing ${expected_code}"
  fi

  if ! grep -q "${expected_protocol_code}" "${output_file}"; then
    fail_step "${step_name}" "${command}" 1 "negative smoke output missing ${expected_protocol_code}"
  fi

  printf "[%s] PASS: %s\n" "$(timestamp)" "${step_name}" >> "${LOG_FILE}"
}

printf "Cowork smoke log: %s\n" "${LOG_FILE}"
printf "CARGO_TARGET_DIR=%s\n" "${CARGO_TARGET_DIR}" >> "${LOG_FILE}"
printf "TMPDIR=%s\n" "${TMPDIR}" >> "${LOG_FILE}"

run_step "format cowork-ledger" \
  cargo fmt -p cowork-ledger --check
run_step "format cowork-runtime" \
  cargo fmt -p cowork-runtime --check
run_step "format cowork-context" \
  cargo fmt -p cowork-context --check
run_step "format cowork-governance" \
  cargo fmt -p cowork-governance --check
run_step "format cowork-protocol" \
  cargo fmt -p cowork-protocol --check
run_step "format cowork-provider-registry" \
  cargo fmt -p cowork-provider-registry --check
run_step "format cowork-plugin-registry" \
  cargo fmt -p cowork-plugin-registry --check
run_step "format cowork-workspace" \
  cargo fmt -p cowork-workspace --check
run_step "format bitfun-cli" \
  cargo fmt -p bitfun-cli --check
run_step "test cowork-ledger" \
  cargo test -p cowork-ledger
run_step "test cowork-runtime" \
  cargo test -p cowork-runtime
run_step "test cowork-runtime subagent failure suite" \
  cargo test -p cowork-runtime subagent_failure_suite
run_step "test cowork-runtime plan repair suite" \
  cargo test -p cowork-runtime plan_repair_suite
run_step "test cowork-context" \
  cargo test -p cowork-context
run_step "test cowork-governance" \
  cargo test -p cowork-governance
run_step "test cowork-protocol" \
  cargo test -p cowork-protocol
run_step "test cowork-provider-registry" \
  cargo test -p cowork-provider-registry
run_step "test cowork-plugin-registry" \
  cargo test -p cowork-plugin-registry
run_step "test cowork-workspace" \
  cargo test -p cowork-workspace
run_step "test bitfun-cli cowork_daemon" \
  cargo test -p bitfun-cli cowork_daemon
run_step "check selected Cowork crates" \
  cargo check -p cowork-ledger -p cowork-runtime -p cowork-context -p cowork-governance -p cowork-protocol -p cowork-provider-registry -p cowork-plugin-registry -p cowork-workspace -p bitfun-cli
run_step "daemon help" \
  cargo run -q -p bitfun-cli -- daemon --help
run_step "daemon start" \
  cargo run -q -p bitfun-cli -- daemon start
run_step "daemon status" \
  cargo run -q -p bitfun-cli -- daemon status
run_step "daemon export" \
  cargo run -q -p bitfun-cli -- daemon export
run_step "daemon export json" \
  cargo run -q -p bitfun-cli -- daemon export --format json
run_step "daemon export markdown" \
  cargo run -q -p bitfun-cli -- daemon export --format markdown
run_step "daemon retrieve task" \
  cargo run -q -p bitfun-cli -- daemon retrieve --kind task --id cowork-smoke-connect
run_step "daemon retrieve checkpoint range" \
  cargo run -q -p bitfun-cli -- daemon retrieve --kind checkpoint --sequence-start 1 --sequence-end 1
run_step "daemon retrieve query" \
  cargo run -q -p bitfun-cli -- daemon retrieve --query protocol
run_negative_smoke "daemon unsupported endpoint" "unsupported_endpoint" "invalid_request" \
  cargo run -q -p bitfun-cli -- daemon start --endpoint unsupported-endpoint
run_negative_smoke "daemon unsupported target" "unsupported_target" "invalid_request" \
  cargo run -q -p bitfun-cli -- daemon start --target unsupported-target
run_negative_smoke "daemon unsupported export format" "unsupported_export_format" "invalid_request" \
  cargo run -q -p bitfun-cli -- daemon export --format yaml
run_negative_smoke "daemon missing history record" "history_record_not_found" "not_found" \
  cargo run -q -p bitfun-cli -- daemon retrieve --kind evidence --id missing-evidence
run_negative_smoke "daemon unsupported retrieval target" "unsupported_retrieval_target" "invalid_request" \
  cargo run -q -p bitfun-cli -- daemon retrieve --kind turn
run_negative_smoke "daemon invalid retrieval range" "invalid_history_range" "invalid_request" \
  cargo run -q -p bitfun-cli -- daemon retrieve --kind checkpoint --sequence-start 2 --sequence-end 1

printf "Cowork smoke passed. Log: %s\n" "${LOG_FILE}"
