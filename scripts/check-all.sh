#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

CHECK_TMP_DIR=""

cleanup() {
  if [[ -n "$CHECK_TMP_DIR" ]]; then
    rm -rf "$CHECK_TMP_DIR"
  fi
}
trap cleanup EXIT

require_command() {
  local command_name="$1"
  local install_hint="$2"

  if ! command -v "$command_name" >/dev/null 2>&1; then
    die "required command '$command_name' was not found. $install_hint"
  fi
}

run_check() {
  local label="$1"
  shift
  log "$label"
  "$@"
}

relative_path() {
  local path="$1"
  printf '%s' "${path#"$ROOT_DIR"/}"
}

check_shell_syntax() {
  local file
  local file_count=0

  log "checking Bash syntax"
  while IFS= read -r -d '' file; do
    if ! bash -n "$file"; then
      die "Bash syntax check failed: $(relative_path "$file")"
    fi
    ((file_count += 1))
  done < <(
    find "$ROOT_DIR" -type f -name '*.sh' \
      -not -path "$ROOT_DIR/.git/*" \
      -not -path "$ROOT_DIR/target/*" \
      -not -path "$ROOT_DIR/dashboard/node_modules/*" \
      -print0 | sort -z
  )

  log "Bash syntax valid for $file_count script(s)"
}

check_shell_lint() {
  local files=()

  while IFS= read -r -d '' file; do
    files+=("$file")
  done < <(
    find "$ROOT_DIR/scripts" -type f -name '*.sh' -print0 | sort -z
  )

  if command -v shellcheck >/dev/null 2>&1; then
    run_check "running ShellCheck for ${#files[@]} script(s)" shellcheck "${files[@]}"
  elif [[ "${CI:-}" == "true" || "${MODELPORT_REQUIRE_SHELLCHECK:-0}" == "1" ]]; then
    die "shellcheck is required in CI/release mode but was not found"
  else
    log "ShellCheck skipped: install shellcheck or set MODELPORT_REQUIRE_SHELLCHECK=1 to enforce it"
  fi
}

npm_has_script() {
  local script_name="$1"

  node -e '
    const scripts = require(process.argv[1]).scripts || {};
    process.exit(Object.prototype.hasOwnProperty.call(scripts, process.argv[2]) ? 0 : 1);
  ' "$ROOT_DIR/dashboard/package.json" "$script_name"
}

prepare_dashboard_dependencies() {
  if [[ "${CI:-}" == "true" || "${MODELPORT_CHECK_NPM_CI:-0}" == "1" || ! -d "$ROOT_DIR/dashboard/node_modules" ]]; then
    run_check "installing locked dashboard dependencies" \
      npm --prefix "$ROOT_DIR/dashboard" ci --no-audit --no-fund
    return
  fi

  if npm --prefix "$ROOT_DIR/dashboard" ls --depth=0 --silent >/dev/null 2>&1; then
    log "using the existing dashboard dependencies (npm dependency check passed)"
    return
  fi

  log "dashboard dependencies are missing or stale; reinstalling from package-lock.json"
  npm --prefix "$ROOT_DIR/dashboard" ci --no-audit --no-fund
}

run_dashboard_checks() {
  prepare_dashboard_dependencies

  if npm_has_script check; then
    run_check "running dashboard checks" \
      npm --prefix "$ROOT_DIR/dashboard" run check
    return
  fi

  for script_name in typecheck lint; do
    if ! npm_has_script "$script_name"; then
      die "dashboard/package.json must define a '$script_name' script"
    fi
    run_check "running dashboard $script_name" \
      npm --prefix "$ROOT_DIR/dashboard" run "$script_name"
  done

  if npm_has_script test; then
    run_check "running dashboard unit tests" \
      npm --prefix "$ROOT_DIR/dashboard" run test
  else
    log "dashboard unit tests skipped: dashboard/package.json does not define a 'test' script"
  fi

  if ! npm_has_script build; then
    die "dashboard/package.json must define a 'build' script"
  fi
  run_check "building dashboard" npm --prefix "$ROOT_DIR/dashboard" run build
}

validate_env_file_shape() {
  local file="$1"

  if ! awk '
    /^[[:space:]]*($|#)/ { next }
    /^[[:space:]]*(export[[:space:]]+)?[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=/ {
      line = $0
      sub(/^[[:space:]]*(export[[:space:]]+)?/, "", line)
      key = line
      sub(/[[:space:]]*=.*/, "", key)
      if (seen[key]++) {
        printf "%s:%d: duplicate environment variable %s\n", FILENAME, FNR, key > "/dev/stderr"
        invalid = 1
      }
      next
    }
    {
      printf "%s:%d: expected KEY=VALUE or a comment\n", FILENAME, FNR > "/dev/stderr"
      invalid = 1
    }
    END { exit invalid }
  ' "$file"; then
    die "environment example format check failed: $(relative_path "$file")"
  fi
}

sanitize_env_example() {
  local source_file="$1"
  local destination_file="$2"

  awk '
    /^[[:space:]]*($|#)/ { print; next }
    {
      line = $0
      sub(/^[[:space:]]*(export[[:space:]]+)?/, "", line)
      key = line
      sub(/[[:space:]]*=.*/, "", key)

      if (key ~ /(API_KEY|AUTH_TOKEN|PASSWORD|SECRET)$/) {
        print key "=ci-validation-secret-" FNR
      } else {
        print line
      }
    }
  ' "$source_file" > "$destination_file"
}

validate_env_example() {
  local source_file="$1"
  local sanitized_file
  sanitized_file="$CHECK_TMP_DIR/$(relative_path "$source_file" | tr '/' '_')"

  run_check "checking environment example syntax: $(relative_path "$source_file")" \
    bash -n "$source_file"
  validate_env_file_shape "$source_file"
  sanitize_env_example "$source_file" "$sanitized_file"

  run_check "loading environment example: $(relative_path "$source_file")" \
    env -i \
      HOME="$CHECK_TMP_DIR/home" \
      PATH="$PATH" \
      MODELPORT_CONFIG="$CHECK_TMP_DIR/no-config.toml" \
      MODELPORT_ENV_FILE="$sanitized_file" \
      MODELPORT_DATABASE_URL="postgres://modelport:ci-validation@db.example:5432/modelport" \
      "$ROOT_DIR/target/debug/model-port" config validate
}

write_config_validation_env() {
  local config_file="$1"
  local destination_file="$2"

  awk -F '"' '
    /^[[:space:]]*(api_key_env|token_env)[[:space:]]*=/ {
      key = $2
      if (key != "" && !seen[key]++) {
        print key "=ci-validation-secret-" FNR
      }
    }
    END { print "MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS=1" }
  ' "$config_file" > "$destination_file"
}

validate_config_examples() {
  local config_env_file
  local config_example
  local env_example
  local config_examples=(
    "$ROOT_DIR/config.example.toml"
    "$ROOT_DIR/deploy/local-inference/modelport.local-qwen.toml"
  )
  local env_examples=(
    "$ROOT_DIR/.env.example"
    "$ROOT_DIR/deploy/docker/modelport.env.example"
    "$ROOT_DIR/deploy/systemd/modelport.env.example"
  )

  CHECK_TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/modelport-check.XXXXXX")"
  mkdir -p "$CHECK_TMP_DIR/home"

  run_check "building the configuration validator" \
    cargo build --locked --quiet --bin model-port

  for env_example in "${env_examples[@]}"; do
    if [[ ! -f "$env_example" ]]; then
      die "missing environment example: $(relative_path "$env_example")"
    fi
    validate_env_example "$env_example"
  done

  for config_example in "${config_examples[@]}"; do
    if [[ ! -f "$config_example" ]]; then
      die "missing configuration example: $(relative_path "$config_example")"
    fi
    config_env_file="$CHECK_TMP_DIR/$(relative_path "$config_example" | tr '/' '_').env"
    write_config_validation_env "$config_example" "$config_env_file"
    run_check "loading configuration example: $(relative_path "$config_example")" \
      env -i \
        HOME="$CHECK_TMP_DIR/home" \
        PATH="$PATH" \
        MODELPORT_CONFIG="$config_example" \
        MODELPORT_ENV_FILE="$config_env_file" \
        MODELPORT_DATABASE_URL="postgres://modelport:ci-validation@db.example:5432/modelport" \
        "$ROOT_DIR/target/debug/model-port" config validate
  done
}

validate_runtime_adapter_examples() {
  local capabilities="$ROOT_DIR/fixtures/runtime-adapters/qwen-llama-cpp-capabilities-v1alpha1.json"
  local compute_inventory="$ROOT_DIR/fixtures/runtime-adapters/qwen-llama-cpp-compute-inventory-v1alpha1.json"
  local invalid="$CHECK_TMP_DIR/invalid-runtime-adapter-capabilities.json"
  local invalid_compute="$CHECK_TMP_DIR/invalid-runtime-adapter-compute-inventory.json"

  run_check "validating the reference Runtime Adapter capabilities" \
    "$ROOT_DIR/target/debug/model-port" runtime-adapter validate "$capabilities"

  run_check "validating the reference Runtime Adapter compute inventory" \
    "$ROOT_DIR/target/debug/model-port" runtime-adapter validate "$compute_inventory"

  printf '%s\n' \
    '{"apiVersion":"runtime.modelport.io/v1alpha1","kind":"RuntimeAdapterCapabilities","metadata":{},"spec":{"operations":[{"operationId":"capabilities.get","method":"POST","path":"/runtime-adapter/v1alpha1/capabilities","sideEffectFree":false}]}}' \
    > "$invalid"
  if "$ROOT_DIR/target/debug/model-port" runtime-adapter validate "$invalid" >/dev/null 2>&1; then
    die "the Runtime Adapter validator accepted a mutating invalid document"
  fi
  log "invalid Runtime Adapter capability document rejected"

  printf '%s\n' \
    '{"apiVersion":"runtime.modelport.io/v1alpha1","kind":"RuntimeAdapterComputeInventory","metadata":{"adapterId":"fixture","snapshotId":"snapshot:invalid","observedAt":"not-rfc3339","source":{"collectorId":"fixture","collectorVersion":"0.1.0"}},"nodes":[]}' \
    > "$invalid_compute"
  if "$ROOT_DIR/target/debug/model-port" runtime-adapter validate "$invalid_compute" >/dev/null 2>&1; then
    die "the Runtime Adapter validator accepted an invalid observation timestamp"
  fi
  log "invalid Runtime Adapter compute inventory rejected"
}

main() {
  cd "$ROOT_DIR"

  require_command bash "Install Bash before running the repository checks."
  require_command cargo "Install the Rust toolchain before running the repository checks."
  require_command rustfmt "Install it with 'rustup component add rustfmt'."
  require_command clippy-driver "Install it with 'rustup component add clippy'."
  require_command node "Install the Node.js version used by the dashboard."
  require_command npm "Install npm before running the dashboard checks."
  require_command awk "Install a POSIX-compatible awk implementation."
  require_command find "Install GNU findutils."
  require_command sort "Install GNU coreutils."
  require_command mktemp "Install GNU coreutils."

  setup_cc_fallback

  check_shell_syntax
  check_shell_lint
  run_check "checking Markdown links" node "$ROOT_DIR/scripts/check-doc-links.mjs"
  run_check "checking Rust formatting" cargo fmt --all -- --check
  run_check "running Rust tests" cargo test --locked --all-targets
  run_check "running Rust clippy" \
    cargo clippy --locked --all-targets --all-features -- -D warnings
  run_dashboard_checks
  validate_config_examples
  validate_runtime_adapter_examples

  log "all repository checks passed"
}

main "$@"
