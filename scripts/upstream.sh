# Shared by the reference scripts: copy paths of thesysdev/openui at a commit.
#
#   fetch_upstream <commit> <dest-dir> <local-checkout-or-empty> <path>...
#
# From a local checkout any commit name works (git archive). From GitHub the
# commit must be a full 40-character hash (or a branch or tag name).
# Prints the full commit hash.

UPSTREAM_URL="https://github.com/thesysdev/openui.git"

fetch_upstream() {
  local commit="$1" dest="$2" local_repo="$3"
  shift 3
  if [[ -n "$local_repo" ]]; then
    git -C "$local_repo" archive "$commit" "$@" | tar -x -C "$dest"
    git -C "$local_repo" rev-parse "$commit"
    return
  fi
  if [[ "$commit" =~ ^[0-9a-f]{7,39}$ ]]; then
    echo "GitHub won't fetch a shortened hash; pass the full 40-character commit." >&2
    return 1
  fi
  local repo
  repo="$(mktemp -d)"
  git -C "$repo" init -q
  git -C "$repo" remote add origin "$UPSTREAM_URL"
  git -C "$repo" sparse-checkout set --no-cone "$@"
  git -C "$repo" fetch -q --depth 1 --filter=blob:none origin "$commit"
  git -C "$repo" checkout -q FETCH_HEAD
  for path in "$@"; do
    if [[ -e "$repo/$path" ]]; then
      mkdir -p "$dest/$(dirname "$path")"
      cp -R "$repo/$path" "$dest/$path"
    fi
  done
  git -C "$repo" rev-parse HEAD
  rm -rf "$repo"
}
