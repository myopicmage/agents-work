#!/usr/bin/env sh

set -eu

INSTALL_SCHEMA="agents-work-install-v1"

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: ./install.sh [options]

Install agents-work, its configuration, and its skill for selected agents.

Options:
  --implementation python|rust  Implementation to install
  --agent codex|claude|both|none
                                 Agent skill target; may be repeated
  --bin-dir PATH                 Command installation directory
  --config-dir PATH              Configuration directory
  --data-dir PATH                Shared-work workspace
  --state-dir PATH               Installer ownership state
  --codex-skills-dir PATH        Codex personal skills directory
  --claude-skills-dir PATH       Claude Code personal skills directory
  --non-interactive              Accept detected defaults without prompts
  -h, --help                     Show this help

Paths must be absolute. Interactive installation offers the same choices.
EOF
}

script_dir=$(CDPATH='' cd -P "$(dirname "$0")" && pwd)

[ -n "${HOME:-}" ] || die "HOME must be set"

xdg_config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
xdg_data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
xdg_state_home=${XDG_STATE_HOME:-"$HOME/.local/state"}
xdg_bin_home=${XDG_BIN_HOME:-"$HOME/.local/bin"}

case "$xdg_config_home" in /*) ;; *) xdg_config_home="$HOME/.config" ;; esac
case "$xdg_data_home" in /*) ;; *) xdg_data_home="$HOME/.local/share" ;; esac
case "$xdg_state_home" in /*) ;; *) xdg_state_home="$HOME/.local/state" ;; esac
case "$xdg_bin_home" in /*) ;; *) xdg_bin_home="$HOME/.local/bin" ;; esac

codex_home=${CODEX_HOME:-"$HOME/.codex"}
case "$codex_home" in /*) ;; *) codex_home="$HOME/.codex" ;; esac

implementation=""
codex_selected=0
claude_selected=0
agents_explicit=0
agents_none=0
non_interactive=0
locations_explicit=0

bin_dir="$xdg_bin_home"
config_dir="$xdg_config_home/agents-work"
data_dir="$xdg_data_home/agents-work"
state_dir="$xdg_state_home/agents-work"
codex_skills_dir="$codex_home/skills"
claude_skills_dir="$HOME/.claude/skills"

select_agent() {
  selected=$1

  case "$selected" in
    codex)
      [ "$agents_none" -eq 0 ] || die "--agent none cannot be combined with another agent"
      codex_selected=1
      ;;
    claude)
      [ "$agents_none" -eq 0 ] || die "--agent none cannot be combined with another agent"
      claude_selected=1
      ;;
    both)
      [ "$agents_none" -eq 0 ] || die "--agent none cannot be combined with another agent"
      codex_selected=1
      claude_selected=1
      ;;
    none)
      [ "$codex_selected" -eq 0 ] && [ "$claude_selected" -eq 0 ] || \
        die "--agent none cannot be combined with another agent"
      agents_none=1
      ;;
    *)
      die "unknown agent: $selected"
      ;;
  esac

  agents_explicit=1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --implementation)
      [ "$#" -ge 2 ] || die "--implementation requires a value"
      implementation=$2
      shift 2
      ;;
    --agent)
      [ "$#" -ge 2 ] || die "--agent requires a value"
      select_agent "$2"
      shift 2
      ;;
    --bin-dir)
      [ "$#" -ge 2 ] || die "--bin-dir requires a value"
      bin_dir=$2
      locations_explicit=1
      shift 2
      ;;
    --config-dir)
      [ "$#" -ge 2 ] || die "--config-dir requires a value"
      config_dir=$2
      locations_explicit=1
      shift 2
      ;;
    --data-dir)
      [ "$#" -ge 2 ] || die "--data-dir requires a value"
      data_dir=$2
      locations_explicit=1
      shift 2
      ;;
    --state-dir)
      [ "$#" -ge 2 ] || die "--state-dir requires a value"
      state_dir=$2
      locations_explicit=1
      shift 2
      ;;
    --codex-skills-dir)
      [ "$#" -ge 2 ] || die "--codex-skills-dir requires a value"
      codex_skills_dir=$2
      locations_explicit=1
      shift 2
      ;;
    --claude-skills-dir)
      [ "$#" -ge 2 ] || die "--claude-skills-dir requires a value"
      claude_skills_dir=$2
      locations_explicit=1
      shift 2
      ;;
    --non-interactive)
      non_interactive=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

expand_home() {
  # shellcheck disable=SC2088
  case "$1" in
    "~/"*) printf '%s/%s\n' "$HOME" "${1#\~/}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

bin_dir=$(expand_home "$bin_dir")
config_dir=$(expand_home "$config_dir")
data_dir=$(expand_home "$data_dir")
state_dir=$(expand_home "$state_dir")
codex_skills_dir=$(expand_home "$codex_skills_dir")
claude_skills_dir=$(expand_home "$claude_skills_dir")

python_available=0
rust_available=0

if command -v python3 >/dev/null 2>&1 && \
  python3 -c 'import sys; raise SystemExit(sys.version_info < (3, 11))' 2>/dev/null; then
  python_available=1
fi

if command -v cargo >/dev/null 2>&1; then
  rust_available=1
fi

if [ -z "$implementation" ]; then
  if [ "$python_available" -eq 1 ]; then
    implementation=python
  elif [ "$rust_available" -eq 1 ]; then
    implementation=rust
  else
    die "Python 3.11+ or a working Cargo installation is required"
  fi

  if [ "$non_interactive" -eq 0 ] && [ -t 0 ]; then
    printf 'Implementation [python/rust] (%s): ' "$implementation"
    IFS= read -r answer || answer=""
    [ -z "$answer" ] || implementation=$answer
  fi
fi

case "$implementation" in
  python)
    [ "$python_available" -eq 1 ] || die "Python 3.11 or newer is required"
    ;;
  rust)
    [ "$rust_available" -eq 1 ] || die "a working Cargo installation is required"
    cargo --version >/dev/null 2>&1 || die "a working Cargo installation is required"
    ;;
  *)
    die "implementation must be python or rust"
    ;;
esac

if [ "$agents_explicit" -eq 0 ]; then
  codex_detected=0
  claude_detected=0

  if command -v codex >/dev/null 2>&1 || [ -d "$codex_home" ]; then
    codex_detected=1
  fi

  if command -v claude >/dev/null 2>&1 || [ -d "$HOME/.claude" ]; then
    claude_detected=1
  fi

  if [ "$codex_detected" -eq 1 ] && [ "$claude_detected" -eq 1 ]; then
    detected_agents=both
  elif [ "$codex_detected" -eq 1 ]; then
    detected_agents=codex
  elif [ "$claude_detected" -eq 1 ]; then
    detected_agents=claude
  else
    detected_agents=none
  fi

  if [ "$non_interactive" -eq 0 ] && [ -t 0 ]; then
    printf 'Install skill for [codex/claude/both/none] (%s): ' "$detected_agents"
    IFS= read -r answer || answer=""
    [ -n "$answer" ] || answer=$detected_agents
    select_agent "$answer"
  else
    select_agent "$detected_agents"
  fi
fi

if [ "$non_interactive" -eq 0 ] && [ -t 0 ] && [ "$locations_explicit" -eq 0 ]; then
  printf 'Use the default XDG and agent locations? [Y/n]: '
  IFS= read -r answer || answer=""

  case "$answer" in
    n|N|no|NO|No)
      printf 'Binary directory (%s): ' "$bin_dir"
      IFS= read -r answer || answer=""
      [ -z "$answer" ] || bin_dir=$(expand_home "$answer")

      printf 'Configuration directory (%s): ' "$config_dir"
      IFS= read -r answer || answer=""
      [ -z "$answer" ] || config_dir=$(expand_home "$answer")

      printf 'Workspace data directory (%s): ' "$data_dir"
      IFS= read -r answer || answer=""
      [ -z "$answer" ] || data_dir=$(expand_home "$answer")

      printf 'Installer state directory (%s): ' "$state_dir"
      IFS= read -r answer || answer=""
      [ -z "$answer" ] || state_dir=$(expand_home "$answer")

      if [ "$codex_selected" -eq 1 ]; then
        printf 'Codex skills directory (%s): ' "$codex_skills_dir"
        IFS= read -r answer || answer=""
        [ -z "$answer" ] || codex_skills_dir=$(expand_home "$answer")
      fi

      if [ "$claude_selected" -eq 1 ]; then
        printf 'Claude Code skills directory (%s): ' "$claude_skills_dir"
        IFS= read -r answer || answer=""
        [ -z "$answer" ] || claude_skills_dir=$(expand_home "$answer")
      fi
      ;;
  esac
fi

validate_path() {
  path_to_validate=$1
  label=$2
  newline='
'
  tab=$(printf '\t')

  case "$path_to_validate" in
    /*) ;;
    *) die "$label must be an absolute path: $path_to_validate" ;;
  esac

  case "$path_to_validate" in
    *'|'*|*"$newline"*|*"$tab"*)
      die "$label contains an unsupported delimiter"
      ;;
  esac
}

validate_path "$bin_dir" "binary directory"
validate_path "$config_dir" "configuration directory"
validate_path "$data_dir" "workspace data directory"
validate_path "$state_dir" "installer state directory"
validate_path "$codex_skills_dir" "Codex skills directory"
validate_path "$claude_skills_dir" "Claude Code skills directory"

reject_inside_data() {
  candidate=$1
  label=$2

  case "$candidate/" in
    "$data_dir/"*)
      die "$label must not be inside the collaboration workspace"
      ;;
  esac
}

reject_inside_data "$bin_dir" "binary directory"
reject_inside_data "$config_dir" "configuration directory"
reject_inside_data "$state_dir" "installer state directory"

if [ "$codex_selected" -eq 1 ]; then
  reject_inside_data "$codex_skills_dir" "Codex skills directory"
fi

if [ "$claude_selected" -eq 1 ]; then
  reject_inside_data "$claude_skills_dir" "Claude Code skills directory"
fi

manifest="$state_dir/install-manifest"

if [ -f "$manifest" ]; then
  printf 'Updating the existing agents-work installation.\n'
  "$script_dir/uninstall.sh" --state-dir "$state_dir" --yes
fi

codex_skill_dir="$codex_skills_dir/agents-work"
claude_skill_dir="$claude_skills_dir/agents-work"
binary_path="$bin_dir/agents-work"
uninstall_command="$bin_dir/agents-work-uninstall"
installed_uninstaller="$config_dir/uninstall.sh"
config_file="$config_dir/config.toml"
protocol_file="$config_dir/PROTOCOL.md"
state_pointer="$config_dir/state-dir"

assert_available() {
  target=$1

  if [ -e "$target" ] || [ -L "$target" ]; then
    die "refusing to replace an unmanaged path: $target"
  fi
}

assert_available "$binary_path"
assert_available "$uninstall_command"
assert_available "$installed_uninstaller"
assert_available "$config_file"
assert_available "$protocol_file"
assert_available "$state_pointer"

if [ "$codex_selected" -eq 1 ]; then
  assert_available "$codex_skill_dir/SKILL.md"
  assert_available "$codex_skill_dir/installation.toml"
  assert_available "$codex_skill_dir/.agents-work-owner"
fi

if [ "$claude_selected" -eq 1 ]; then
  assert_available "$claude_skill_dir/SKILL.md"
  assert_available "$claude_skill_dir/installation.toml"
  assert_available "$claude_skill_dir/.agents-work-owner"
fi

mkdir -p "$bin_dir" "$config_dir" "$data_dir" "$state_dir"

if [ "$codex_selected" -eq 1 ]; then
  mkdir -p "$codex_skill_dir"
fi

if [ "$claude_selected" -eq 1 ]; then
  mkdir -p "$claude_skill_dir"
fi

temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/agents-work-install.XXXXXX")
installing=1

cleanup() {
  status=$?
  trap - EXIT HUP INT TERM

  if [ "$installing" -eq 1 ] && [ -f "$manifest" ]; then
    "$script_dir/uninstall.sh" --state-dir "$state_dir" --force --yes >/dev/null 2>&1 || true
  fi

  rm -rf "$temp_dir"
  exit "$status"
}

trap cleanup EXIT HUP INT TERM

printf '%s\n' "$INSTALL_SCHEMA" > "$manifest"

checksum_fields() {
  checksum_output=$(cksum < "$1")
  checksum=${checksum_output%% *}
  size=${checksum_output#* }
  size=${size%% *}
  printf '%s|%s' "$checksum" "$size"
}

record_file() {
  fields=$(checksum_fields "$1")
  printf 'file|%s|%s\n' "$fields" "$1" >> "$manifest"
}

install_owned_file() {
  source_file=$1
  destination=$2
  mode=$3

  install -m "$mode" "$source_file" "$destination"
  record_file "$destination"
}

toml_quote() {
  escaped=$(printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')
  printf '"%s"' "$escaped"
}

if [ "$implementation" = python ]; then
  install_owned_file "$script_dir/python/agents_work.py" "$binary_path" 755
else
  rust_target="$temp_dir/rust-target"
  CARGO_TARGET_DIR="$rust_target" \
    cargo build --locked --release --manifest-path "$script_dir/rust/Cargo.toml"
  install_owned_file "$rust_target/release/agents-work" "$binary_path" 755
fi

install_owned_file "$script_dir/uninstall.sh" "$installed_uninstaller" 755
install_owned_file "$script_dir/PROTOCOL.md" "$protocol_file" 644

agents_toml=""
if [ "$codex_selected" -eq 1 ] && [ "$claude_selected" -eq 1 ]; then
  agents_toml='["codex", "claude"]'
elif [ "$codex_selected" -eq 1 ]; then
  agents_toml='["codex"]'
elif [ "$claude_selected" -eq 1 ]; then
  agents_toml='["claude"]'
else
  agents_toml='[]'
fi

{
  printf 'schema_version = 1\n'
  printf 'implementation = %s\n' "$(toml_quote "$implementation")"
  printf 'binary = %s\n' "$(toml_quote "$binary_path")"
  printf 'workspace = %s\n' "$(toml_quote "$data_dir")"
  printf 'protocol = %s\n' "$(toml_quote "$protocol_file")"
  printf 'agents = %s\n' "$agents_toml"
} > "$temp_dir/config.toml"
install_owned_file "$temp_dir/config.toml" "$config_file" 644

printf '%s\n' "$state_dir" > "$temp_dir/state-dir"
install_owned_file "$temp_dir/state-dir" "$state_pointer" 644

quoted_uninstaller=$(printf '%s' "$installed_uninstaller" | sed "s/'/'\\\\''/g")
{
  printf '#!/usr/bin/env sh\n\n'
  printf "exec '%s' \"\$@\"\n" "$quoted_uninstaller"
} > "$temp_dir/agents-work-uninstall"
install_owned_file "$temp_dir/agents-work-uninstall" "$uninstall_command" 755

install_skill() {
  skill_dir=$1

  install_owned_file "$script_dir/skills/agents-work/SKILL.md" "$skill_dir/SKILL.md" 644
  printf 'schema_version = 1\nconfig = %s\n' \
    "$(toml_quote "$config_file")" > "$temp_dir/installation.toml"
  install_owned_file "$temp_dir/installation.toml" "$skill_dir/installation.toml" 644
  printf '%s\n' "$INSTALL_SCHEMA" > "$temp_dir/skill-owner"
  install_owned_file "$temp_dir/skill-owner" "$skill_dir/.agents-work-owner" 644
}

if [ "$codex_selected" -eq 1 ]; then
  install_skill "$codex_skill_dir"
fi

if [ "$claude_selected" -eq 1 ]; then
  install_skill "$claude_skill_dir"
fi

printf 'data|||%s\n' "$data_dir" >> "$manifest"

if [ "$codex_selected" -eq 1 ]; then
  printf 'dir|||%s\n' "$codex_skill_dir" >> "$manifest"
fi

if [ "$claude_selected" -eq 1 ]; then
  printf 'dir|||%s\n' "$claude_skill_dir" >> "$manifest"
fi

{
  printf 'dir|||%s\n' "$config_dir"
  printf 'dir|||%s\n' "$bin_dir"
  printf 'dir|||%s\n' "$state_dir"
} >> "$manifest"

installing=0

printf '\nInstalled agents-work (%s).\n' "$implementation"
printf '  command:   %s\n' "$binary_path"
printf '  config:    %s\n' "$config_file"
printf '  workspace: %s\n' "$data_dir"

if [ "$codex_selected" -eq 1 ]; then
  printf '  Codex skill: %s\n' "$codex_skill_dir"
fi

if [ "$claude_selected" -eq 1 ]; then
  printf '  Claude skill: %s\n' "$claude_skill_dir"
fi

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) printf '\nAdd %s to PATH to use agents-work directly.\n' "$bin_dir" ;;
esac

printf 'Uninstall with: %s\n' "$uninstall_command"
