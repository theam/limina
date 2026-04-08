#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLAUDE_HOME_DIR="${CLAUDE_HOME:-$HOME/.claude}"
CODEX_HOME_DIR="${CODEX_HOME:-$HOME/.codex}"

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
}

main() {
  local claude_only=0
  local codex_only=0

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
    install_target "${CLAUDE_HOME_DIR}" "Claude Code"
    exit 0
  fi

  if [[ "${codex_only}" -eq 1 ]]; then
    install_target "${CODEX_HOME_DIR}" "Codex"
    exit 0
  fi

  if [[ -d "${CLAUDE_HOME_DIR}" || ! -d "${CODEX_HOME_DIR}" ]]; then
    install_target "${CLAUDE_HOME_DIR}" "Claude Code"
  fi

  if [[ -d "${CODEX_HOME_DIR}" || ! -d "${CLAUDE_HOME_DIR}" ]]; then
    install_target "${CODEX_HOME_DIR}" "Codex"
  fi
}

main "$@"
