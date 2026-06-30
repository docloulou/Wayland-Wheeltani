#!/usr/bin/env bash
#
# Publishes the Markdown pages under wiki/ to the project's GitHub wiki.
#
# The GitHub wiki is a separate git repository
# (<repo>.wiki.git). This script clones it, mirrors the local wiki/ folder into
# it, and pushes. Requires push access (the wiki must be enabled in the repo
# settings; create the first page once from the GitHub UI if the clone fails).
#
# Usage:
#   scripts/publish-wiki.sh [--dry-run] [--remote <git-url>]
set -euo pipefail

root_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
wiki_src="$root_dir/wiki"
remote="https://github.com/docloulou/Wayland-Wheeltani.wiki.git"
dry_run=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) dry_run=true ;;
    --remote) shift; [[ $# -gt 0 ]] || { echo "error: --remote needs a value" >&2; exit 1; }; remote="$1" ;;
    -h | --help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; exit 1 ;;
  esac
  shift
done

[[ -d "$wiki_src" ]] || { echo "error: missing $wiki_src" >&2; exit 1; }

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

echo "Cloning wiki: $remote"
if ! git clone --depth 1 "$remote" "$work_dir/wiki" 2>/dev/null; then
  cat >&2 <<MSG
error: could not clone the wiki repository.

Enable the wiki for the repo and create its first page once from the GitHub UI
(Wiki tab -> Create the first page), then re-run this script. Alternatively pass
an explicit URL with --remote.
MSG
  exit 1
fi

# Mirror the local pages into the wiki checkout (Markdown only).
find "$work_dir/wiki" -maxdepth 1 -type f -name '*.md' -delete
cp "$wiki_src"/*.md "$work_dir/wiki/"

cd "$work_dir/wiki"
if git diff --quiet && git diff --cached --quiet; then
  echo "Wiki already up to date; nothing to publish."
  exit 0
fi

git add -A
echo "Pages to publish:"
git status --short

if [[ "$dry_run" == true ]]; then
  echo "Dry run: not committing or pushing."
  exit 0
fi

git commit -m "Sync wiki from wiki/ ($(date -u +%Y-%m-%dT%H:%M:%SZ))"
git push
echo "Wiki published."
