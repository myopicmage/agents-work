#!/usr/bin/env sh

set -eu

die() {
  printf 'test failure: %s\n' "$*" >&2
  exit 1
}

assert_file() {
  [ -f "$1" ] || die "expected file: $1"
}

assert_executable() {
  [ -x "$1" ] || die "expected executable: $1"
}

assert_absent() {
  if [ -e "$1" ] || [ -L "$1" ]; then
    die "expected path to be absent: $1"
  fi
}

assert_contains() {
  grep -F "$2" "$1" >/dev/null || die "expected $1 to contain: $2"
}

usage() {
  cat <<'EOF'
Usage: tests/install_test.sh [--implementation python|rust|all]
EOF
}

implementation=all

while [ "$#" -gt 0 ]; do
  case "$1" in
    --implementation)
      [ "$#" -ge 2 ] || die "--implementation requires a value"
      implementation=$2
      shift 2
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

case "$implementation" in
  python|rust|all) ;;
  *) die "implementation must be python, rust, or all" ;;
esac

repo_root=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/agents-work-install-test.XXXXXX")

cleanup() {
  status=$?
  trap - EXIT HUP INT TERM
  rm -rf "$test_root"
  exit "$status"
}

trap cleanup EXIT HUP INT TERM

test_python_install() {
  home="$test_root/python-home"
  mkdir -p "$home"

  HOME="$home" \
  CODEX_HOME="$home/codex" \
  XDG_CONFIG_HOME="$home/xdg/config" \
  XDG_DATA_HOME="$home/xdg/data" \
  XDG_STATE_HOME="$home/xdg/state" \
  XDG_BIN_HOME="$home/bin" \
    "$repo_root/install.sh" \
      --non-interactive \
      --implementation python \
      --agent both >/dev/null

  command="$home/bin/agents-work"
  uninstaller="$home/bin/agents-work-uninstall"
  config="$home/xdg/config/agents-work/config.toml"
  data="$home/xdg/data/agents-work"
  state="$home/xdg/state/agents-work/install-manifest"
  codex_skill="$home/codex/skills/agents-work/SKILL.md"
  claude_skill="$home/.claude/skills/agents-work/SKILL.md"

  assert_executable "$command"
  assert_executable "$uninstaller"
  assert_file "$config"
  assert_file "$state"
  assert_file "$codex_skill"
  assert_file "$claude_skill"
  assert_contains "$config" 'implementation = "python"'
  assert_contains "$config" "workspace = \"$data\""
  assert_contains "$home/codex/skills/agents-work/installation.toml" \
    "config = \"$config\""
  "$command" --help >/dev/null

  printf 'preserve me\n' > "$data/case-artifact.md"

  HOME="$home" \
  CODEX_HOME="$home/codex" \
  XDG_CONFIG_HOME="$home/xdg/config" \
  XDG_DATA_HOME="$home/xdg/data" \
  XDG_STATE_HOME="$home/xdg/state" \
  XDG_BIN_HOME="$home/bin" \
    "$repo_root/install.sh" \
      --non-interactive \
      --implementation python \
      --agent both >/dev/null

  assert_file "$data/case-artifact.md"
  printf '\n# locally modified\n' >> "$codex_skill"

  if HOME="$home" "$uninstaller" >/dev/null 2>&1; then
    die "uninstall should reject a modified owned skill"
  fi

  assert_executable "$command"
  assert_file "$config"
  assert_file "$codex_skill"

  HOME="$home" "$uninstaller" --force >/dev/null 2>&1

  assert_absent "$command"
  assert_absent "$uninstaller"
  assert_absent "$config"
  assert_absent "$codex_skill"
  assert_absent "$claude_skill"
  assert_absent "$state"
  assert_file "$data/case-artifact.md"
}

test_custom_paths_and_purge() {
  home="$test_root/custom-home"
  custom_root="$test_root/custom path's"
  bin="$custom_root/bin"
  config="$custom_root/config"
  data="$custom_root/data"
  state="$custom_root/state"
  mkdir -p "$home"

  HOME="$home" "$repo_root/install.sh" \
    --non-interactive \
    --implementation python \
    --agent none \
    --bin-dir "$bin" \
    --config-dir "$config" \
    --data-dir "$data" \
    --state-dir "$state" >/dev/null

  assert_executable "$bin/agents-work"
  assert_file "$config/config.toml"
  assert_file "$state/install-manifest"
  HOME="$home" "$bin/agents-work-uninstall" --help >/dev/null
  python3 -c 'import pathlib, sys, tomllib; tomllib.loads(pathlib.Path(sys.argv[1]).read_text())' \
    "$config/config.toml"
  printf 'delete me\n' > "$data/case-artifact.md"

  if printf 'no\n' | HOME="$home" "$repo_root/uninstall.sh" \
    --state-dir "$state" \
    --purge-data >/dev/null 2>&1; then
    die "data purge should require the exact workspace path"
  fi

  assert_executable "$bin/agents-work"
  assert_file "$data/case-artifact.md"

  confirmed_data=$(CDPATH='' cd -P "$data" && pwd)
  printf '%s\n' "$confirmed_data" | HOME="$home" "$repo_root/uninstall.sh" \
    --state-dir "$state" \
    --purge-data >/dev/null

  assert_absent "$bin/agents-work"
  assert_absent "$data"
  assert_absent "$state/install-manifest"
}

test_unmanaged_collision() {
  home="$test_root/collision-home"
  bin="$home/bin"
  mkdir -p "$bin"
  printf 'mine\n' > "$bin/agents-work"

  if HOME="$home" XDG_BIN_HOME="$bin" \
    "$repo_root/install.sh" \
      --non-interactive \
      --implementation python \
      --agent none >/dev/null 2>&1; then
    die "install should reject an unmanaged binary"
  fi

  assert_contains "$bin/agents-work" "mine"
}

test_failed_install_rolls_back() {
  home="$test_root/rollback-home"
  fake_bin="$test_root/rollback-tools"
  bin="$home/bin"
  config="$home/config"
  data="$home/data"
  state="$home/state"
  mkdir -p "$home" "$fake_bin"

  {
    printf '#!/usr/bin/env sh\n'
    # shellcheck disable=SC2016
    printf 'if [ "$1" = "--version" ]; then exit 0; fi\n'
    printf 'exit 1\n'
  } > "$fake_bin/cargo"
  chmod +x "$fake_bin/cargo"

  if HOME="$home" PATH="$fake_bin:$PATH" "$repo_root/install.sh" \
    --non-interactive \
    --implementation rust \
    --agent none \
    --bin-dir "$bin" \
    --config-dir "$config" \
    --data-dir "$data" \
    --state-dir "$state" >/dev/null 2>&1; then
    die "install should fail when the selected compiler fails"
  fi

  assert_absent "$bin/agents-work"
  assert_absent "$config/config.toml"
  assert_absent "$state/install-manifest"
}

test_rust_install() {
  home="$test_root/rust-home"
  mkdir -p "$home"

  HOME="$home" \
  XDG_CONFIG_HOME="$home/xdg/config" \
  XDG_DATA_HOME="$home/xdg/data" \
  XDG_STATE_HOME="$home/xdg/state" \
  XDG_BIN_HOME="$home/bin" \
    "$repo_root/install.sh" \
      --non-interactive \
      --implementation rust \
      --agent none >/dev/null

  assert_executable "$home/bin/agents-work"
  assert_contains "$home/xdg/config/agents-work/config.toml" \
    'implementation = "rust"'
  "$home/bin/agents-work" --help >/dev/null
  HOME="$home" "$home/bin/agents-work-uninstall" >/dev/null
  assert_absent "$home/bin/agents-work"
}

if [ "$implementation" = python ] || [ "$implementation" = all ]; then
  test_python_install
  test_custom_paths_and_purge
  test_unmanaged_collision
  test_failed_install_rolls_back
fi

if [ "$implementation" = rust ] || [ "$implementation" = all ]; then
  test_rust_install
fi

printf 'install and uninstall tests passed (%s)\n' "$implementation"
