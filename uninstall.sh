#!/usr/bin/env sh

set -eu

INSTALL_SCHEMA="agents-work-install-v1"

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: agents-work-uninstall [options]

Remove files owned by the agents-work installer. Collaboration data is kept
unless --purge-data is explicitly selected.

Options:
  --state-dir PATH  Installer state directory
  --purge-data      Also delete the configured collaboration workspace
  --force           Remove installer-owned files even when modified
  --yes             Skip the --purge-data confirmation
  -h, --help        Show this help
EOF
}

[ -n "${HOME:-}" ] || die "HOME must be set"

script_dir=$(CDPATH='' cd -P "$(dirname "$0")" && pwd)
xdg_state_home=${XDG_STATE_HOME:-"$HOME/.local/state"}
case "$xdg_state_home" in /*) ;; *) xdg_state_home="$HOME/.local/state" ;; esac

state_dir="$xdg_state_home/agents-work"
state_explicit=0
purge_data=0
force=0
yes=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --state-dir)
      [ "$#" -ge 2 ] || die "--state-dir requires a value"
      state_dir=$2
      state_explicit=1
      shift 2
      ;;
    --purge-data)
      purge_data=1
      shift
      ;;
    --force)
      force=1
      shift
      ;;
    --yes)
      yes=1
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

if [ "$state_explicit" -eq 0 ] && [ -f "$script_dir/state-dir" ]; then
  IFS= read -r state_dir < "$script_dir/state-dir" || die "cannot read installed state pointer"
fi

case "$state_dir" in
  /*) ;;
  *) die "state directory must be an absolute path: $state_dir" ;;
esac

manifest="$state_dir/install-manifest"
[ -f "$manifest" ] || die "installation manifest not found: $manifest"

IFS= read -r header < "$manifest" || die "installation manifest is empty"
[ "$header" = "$INSTALL_SCHEMA" ] || die "unsupported installation manifest: $header"

checksum_fields() {
  checksum_output=$(cksum < "$1")
  checksum=${checksum_output%% *}
  size=${checksum_output#* }
  size=${size%% *}
  printf '%s|%s' "$checksum" "$size"
}

modified=0
data_dir=""
line_number=0

while IFS='|' read -r kind checksum size path; do
  line_number=$((line_number + 1))

  if [ "$line_number" -eq 1 ]; then
    continue
  fi

  case "$kind" in
    file)
      case "$path" in /*) ;; *) die "manifest contains a non-absolute file path" ;; esac

      if [ -e "$path" ] || [ -L "$path" ]; then
        actual=$(checksum_fields "$path")
        if [ "$actual" != "$checksum|$size" ]; then
          printf 'modified installer-owned file: %s\n' "$path" >&2
          modified=1
        fi
      fi
      ;;
    data)
      case "$path" in /*) ;; *) die "manifest contains a non-absolute data path" ;; esac
      data_dir=$path
      ;;
    dir)
      case "$path" in /*) ;; *) die "manifest contains a non-absolute directory path" ;; esac
      ;;
    "") ;;
    *) die "unknown manifest entry: $kind" ;;
  esac
done < "$manifest"

if [ "$modified" -eq 1 ] && [ "$force" -eq 0 ]; then
  die "uninstall made no changes; rerun with --force to remove modified owned files"
fi

if [ "$purge_data" -eq 1 ]; then
  [ -n "$data_dir" ] || die "manifest does not name a data directory"

  if [ -d "$data_dir" ]; then
    data_dir=$(CDPATH='' cd -P "$data_dir" && pwd)
  fi

  case "$data_dir" in
    /|"$HOME"|"$HOME/"|/Users|/Users/|/home|/home/|/root|/root/|/tmp|/tmp/|/private/tmp|/private/tmp/|/var/tmp|/var/tmp/)
      die "refusing to purge unsafe data path: $data_dir"
      ;;
  esac

  if [ -L "$data_dir" ]; then
    die "refusing to purge a symlinked data directory: $data_dir"
  fi

  if [ "$yes" -eq 0 ]; then
    printf 'Delete every collaboration artifact under %s?\n' "$data_dir"
    printf 'Type the full path to confirm: '
    IFS= read -r answer || answer=""
    [ "$answer" = "$data_dir" ] || die "data purge cancelled"
  fi
fi

line_number=0
while IFS='|' read -r kind checksum size path; do
  line_number=$((line_number + 1))

  if [ "$line_number" -eq 1 ]; then
    continue
  fi

  case "$kind" in
    file)
      if [ -e "$path" ] || [ -L "$path" ]; then
        rm -f "$path"
        printf 'removed %s\n' "$path"
      fi
      ;;
    dir)
      rmdir "$path" 2>/dev/null || true
      ;;
  esac
done < "$manifest"

if [ "$purge_data" -eq 1 ] && [ -e "$data_dir" ]; then
  rm -rf "$data_dir"
  printf 'removed data %s\n' "$data_dir"
elif [ -n "$data_dir" ]; then
  printf 'preserved data %s\n' "$data_dir"
fi

rm -f "$manifest"
rmdir "$state_dir" 2>/dev/null || true

printf 'agents-work uninstalled.\n'
