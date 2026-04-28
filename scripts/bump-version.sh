#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/bump-version.sh [patch|minor|major|--version X.Y.Z] [--dry-run] [--no-check]

Interactively bumps the Wayland-Wheeltani workspace version when no bump type is
provided. The script updates:

  - Cargo.toml [workspace.package].version
  - crates/middle-scroll-linux/Cargo.toml middle-scroll-core dependency version
  - Cargo.lock through cargo check -p wayland-wheeltani

Options:
  patch, --patch       Increment X.Y.Z to X.Y.(Z+1)
  minor, --minor       Increment X.Y.Z to X.(Y+1).0
  major, --major       Increment X.Y.Z to (X+1).0.0
  --version X.Y.Z      Set an explicit version manually
  --dry-run            Show the selected bump without editing files
  --no-check           Do not run cargo check after editing files
  -h, --help           Show this help
USAGE
}

fail() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

root_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
workspace_toml="$root_dir/Cargo.toml"
linux_toml="$root_dir/crates/middle-scroll-linux/Cargo.toml"

requested_bump=""
manual_version=""
dry_run=false
run_check=true

while [[ $# -gt 0 ]]; do
  case "$1" in
    patch | --patch)
      requested_bump="patch"
      ;;
    minor | --minor)
      requested_bump="minor"
      ;;
    major | --major)
      requested_bump="major"
      ;;
    --version)
      shift
      [[ $# -gt 0 ]] || fail '--version requires a value'
      manual_version="$1"
      ;;
    --dry-run)
      dry_run=true
      ;;
    --no-check)
      run_check=false
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
  shift
done

[[ -f "$workspace_toml" ]] || fail "missing $workspace_toml"
[[ -f "$linux_toml" ]] || fail "missing $linux_toml"

if [[ -n "$requested_bump" && -n "$manual_version" ]]; then
  fail 'choose either patch/minor/major or --version, not both'
fi

validate_version() {
  local version="$1"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?$ ]]
}

read_workspace_version() {
  local line in_workspace_package=false

  while IFS= read -r line || [[ -n "$line" ]]; do
    if [[ "$line" == '[workspace.package]' ]]; then
      in_workspace_package=true
      continue
    fi

    if [[ "$in_workspace_package" == true && "$line" =~ ^\[.*\]$ ]]; then
      break
    fi

    if [[ "$in_workspace_package" == true && "$line" =~ ^[[:space:]]*version[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
      printf '%s\n' "${BASH_REMATCH[1]}"
      return 0
    fi
  done < "$workspace_toml"

  return 1
}

increment_version() {
  local current="$1"
  local bump="$2"

  if [[ ! "$current" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
    fail "automatic $bump bump requires a stable X.Y.Z version; choose --version manually"
  fi

  local major="${BASH_REMATCH[1]}"
  local minor="${BASH_REMATCH[2]}"
  local patch="${BASH_REMATCH[3]}"

  case "$bump" in
    patch)
      patch=$((patch + 1))
      ;;
    minor)
      minor=$((minor + 1))
      patch=0
      ;;
    major)
      major=$((major + 1))
      minor=0
      patch=0
      ;;
    *)
      fail "unknown bump type: $bump"
      ;;
  esac

  printf '%s.%s.%s\n' "$major" "$minor" "$patch"
}

prompt_for_version() {
  local current="$1"
  local patch_version minor_version major_version choice version

  patch_version="$(increment_version "$current" patch)"
  minor_version="$(increment_version "$current" minor)"
  major_version="$(increment_version "$current" major)"

  while true; do
    cat >&2 <<MENU
Current version: $current

Choose the next version:
  1) patch  -> $patch_version
  2) minor  -> $minor_version
  3) major  -> $major_version
  4) manual
MENU
    read -r -p 'Choice [1-4]: ' choice

    case "$choice" in
      1 | patch)
        printf '%s\n' "$patch_version"
        return 0
        ;;
      2 | minor)
        printf '%s\n' "$minor_version"
        return 0
        ;;
      3 | major)
        printf '%s\n' "$major_version"
        return 0
        ;;
      4 | manual)
        while true; do
          read -r -p 'Manual version (for example 1.2.3 or 1.2.3-beta.1): ' version
          if validate_version "$version"; then
            printf '%s\n' "$version"
            return 0
          fi
          printf 'Invalid version. Expected X.Y.Z or X.Y.Z-prerelease.\n' >&2
        done
        ;;
      *)
        printf 'Invalid choice. Pick 1, 2, 3, or 4.\n' >&2
        ;;
    esac
  done
}

replace_workspace_version() {
  local new_version="$1"
  local tmp line in_workspace_package=false replaced=false

  tmp="$(mktemp)"
  while IFS= read -r line || [[ -n "$line" ]]; do
    if [[ "$line" == '[workspace.package]' ]]; then
      in_workspace_package=true
      printf '%s\n' "$line" >> "$tmp"
      continue
    fi

    if [[ "$in_workspace_package" == true && "$line" =~ ^\[.*\]$ ]]; then
      in_workspace_package=false
    fi

    if [[ "$in_workspace_package" == true && "$replaced" == false && "$line" =~ ^([[:space:]]*)version([[:space:]]*)=([[:space:]]*)\"[^\"]+\"(.*)$ ]]; then
      printf '%sversion%s=%s"%s"%s\n' \
        "${BASH_REMATCH[1]}" \
        "${BASH_REMATCH[2]}" \
        "${BASH_REMATCH[3]}" \
        "$new_version" \
        "${BASH_REMATCH[4]}" >> "$tmp"
      replaced=true
    else
      printf '%s\n' "$line" >> "$tmp"
    fi
  done < "$workspace_toml"

  [[ "$replaced" == true ]] || {
    rm -f "$tmp"
    fail 'could not replace [workspace.package].version in Cargo.toml'
  }

  mv "$tmp" "$workspace_toml"
}

replace_core_dependency_version() {
  local new_version="$1"
  local tmp line replaced=false

  tmp="$(mktemp)"
  while IFS= read -r line || [[ -n "$line" ]]; do
    if [[ "$replaced" == false && "$line" == *'middle-scroll-core'* && "$line" == *'path = "../middle-scroll-core"'* && "$line" =~ ^(.*version[[:space:]]*=[[:space:]]*)\"[^\"]+\"(.*)$ ]]; then
      printf '%s"%s"%s\n' "${BASH_REMATCH[1]}" "$new_version" "${BASH_REMATCH[2]}" >> "$tmp"
      replaced=true
    else
      printf '%s\n' "$line" >> "$tmp"
    fi
  done < "$linux_toml"

  [[ "$replaced" == true ]] || {
    rm -f "$tmp"
    fail 'could not replace middle-scroll-core dependency version'
  }

  mv "$tmp" "$linux_toml"
}

current_version="$(read_workspace_version)" || fail 'could not read [workspace.package].version'
validate_version "$current_version" || fail "current version is not supported: $current_version"

if [[ -n "$manual_version" ]]; then
  validate_version "$manual_version" || fail "invalid version: $manual_version"
  new_version="$manual_version"
elif [[ -n "$requested_bump" ]]; then
  new_version="$(increment_version "$current_version" "$requested_bump")"
else
  new_version="$(prompt_for_version "$current_version")"
fi

if [[ "$new_version" == "$current_version" ]]; then
  fail "new version is the same as current version: $new_version"
fi

printf 'Bumping version: %s -> %s\n' "$current_version" "$new_version"

if [[ "$dry_run" == true ]]; then
  cat <<DRYRUN
Dry run only. Would update:
  - Cargo.toml [workspace.package].version
  - crates/middle-scroll-linux/Cargo.toml middle-scroll-core dependency version
DRYRUN
  if [[ "$run_check" == true ]]; then
    printf '  - Cargo.lock via cargo check -p wayland-wheeltani\n'
  else
    printf 'Would skip cargo check because --no-check was set.\n'
  fi
  exit 0
fi

replace_workspace_version "$new_version"
replace_core_dependency_version "$new_version"

if [[ "$run_check" == true ]]; then
  (
    cd "$root_dir"
    cargo check -p wayland-wheeltani
  )
else
  printf 'Skipped cargo check because --no-check was set.\n'
fi

cat <<DONE
Version bumped to $new_version.

Updated files:
  - Cargo.toml
  - crates/middle-scroll-linux/Cargo.toml
  - Cargo.lock, if Cargo needed to refresh it
DONE
