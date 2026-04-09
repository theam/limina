#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLAUDE_HOME_DIR="${CLAUDE_HOME:-$HOME/.claude}"
CODEX_HOME_DIR="${CODEX_HOME:-$HOME/.codex}"
TELEMETRY_SCRIPT="${ROOT_DIR}/scripts/telemetry.py"

usage() {
  cat <<'EOF'
Usage: bash scripts/install_skills.sh [--claude-only | --codex-only]

Install the repo-bundled Limina skills into the local Claude Code and/or Codex
skill directories by symlinking back to this repository.
EOF
}

install_target() {
  local home_dir="$1"
  local label="$2"
  local runtime_family="$3"
  local target_dir="${home_dir}/skills"
  local installed=0

  mkdir -p "${target_dir}"

  ln -sfn "${ROOT_DIR}/skill" "${target_dir}/limina"
  echo "Installed limina -> ${target_dir}/limina (${label})"
  installed=$((installed + 1))

  for skill_path in "${ROOT_DIR}/skills"/*; do
    [[ -d "${skill_path}" ]] || continue
    local skill_name
    skill_name="$(basename "${skill_path}")"
    ln -sfn "${skill_path}" "${target_dir}/${skill_name}"
    echo "Installed ${skill_name} -> ${target_dir}/${skill_name} (${label})"
    installed=$((installed + 1))
  done

  echo "Installed ${installed} skill(s) for ${label}."
  if [[ -f "${TELEMETRY_SCRIPT}" ]]; then
    python3 "${TELEMETRY_SCRIPT}" emit limina_skills_installed \
      --runtime-family "${runtime_family}" \
      --emitter install_skills \
      --property "count_total_installed=${installed}" >/dev/null 2>&1 || true
  fi
}

flush_telemetry() {
  if [[ -f "${TELEMETRY_SCRIPT}" ]]; then
    python3 "${TELEMETRY_SCRIPT}" flush >/dev/null 2>&1 || true
  fi
}

main() {
  local claude_only=0
  local codex_only=0

  if [[ -f "${TELEMETRY_SCRIPT}" ]]; then
    python3 "${TELEMETRY_SCRIPT}" ensure-consent --source install_skills || true
  fi

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --claude-only)
        claude_only=1
        ;;
      --codex-only)
        codex_only=1
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      *)
        echo "Unknown argument: $1" >&2
        usage >&2
        exit 1
        ;;
    esac
    shift
  done

  if [[ "${claude_only}" -eq 1 && "${codex_only}" -eq 1 ]]; then
    echo "Choose at most one of --claude-only or --codex-only." >&2
    exit 1
  fi

  if [[ "${claude_only}" -eq 1 ]]; then
    install_target "${CLAUDE_HOME_DIR}" "Claude Code" "claude"
    flush_telemetry
    exit 0
  fi

  if [[ "${codex_only}" -eq 1 ]]; then
    install_target "${CODEX_HOME_DIR}" "Codex" "codex"
    flush_telemetry
    exit 0
  fi

  if [[ -d "${CLAUDE_HOME_DIR}" || ! -d "${CODEX_HOME_DIR}" ]]; then
    install_target "${CLAUDE_HOME_DIR}" "Claude Code" "claude"
  fi

  if [[ -d "${CODEX_HOME_DIR}" || ! -d "${CLAUDE_HOME_DIR}" ]]; then
    install_target "${CODEX_HOME_DIR}" "Codex" "codex"
  fi

  flush_telemetry
}

main "$@"
