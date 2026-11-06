# Sourced by scripts/release-mac.sh and scripts/release-android.sh so the two
# can run at the same time without clobbering each other (card #613). macOS
# has no flock(1), so a lock is a directory: mkdir either makes it (ours) or
# fails (someone else's), atomically. The holder's pid is in it, so a lock
# left by a killed run is taken over instead of waited on forever.
#
#   release_lock <dir> <what>   wait for the lock, then hold it
#   release_unlock <dir>        let it go (an exit lets go of every held lock)

RELEASE_LOCKS=()

release_lock() {
  local dir="$1" what="$2" holder="" said=""
  mkdir -p "$(dirname "$dir")"
  until mkdir "$dir" 2>/dev/null; do
    holder="$(cat "$dir/pid" 2>/dev/null || true)"
    if [[ -n "$holder" ]] && ! kill -0 "$holder" 2>/dev/null; then
      # Read again just before removing, so a lock a third run took in the
      # meantime is left alone.
      if [[ "$(cat "$dir/pid" 2>/dev/null || true)" == "$holder" ]]; then
        echo "taking over $dir from pid $holder, which is gone" >&2
        rm -rf "$dir"
      fi
      continue
    fi
    if [[ -z "$said" ]]; then
      echo "waiting for $(cat "$dir/who" 2>/dev/null || echo "another release script") (pid ${holder:-?}) to finish $what ($dir)" >&2
      said=1
    fi
    sleep 2
  done
  echo "$$" >"$dir/pid"
  echo "$(basename "$0")" >"$dir/who"
  RELEASE_LOCKS+=("$dir")
}

release_unlock() {
  local dir="$1" kept=() d
  rm -rf "$dir"
  for d in ${RELEASE_LOCKS[@]+"${RELEASE_LOCKS[@]}"}; do
    [[ "$d" == "$dir" ]] || kept+=("$d")
  done
  RELEASE_LOCKS=(${kept[@]+"${kept[@]}"})
}

release_unlock_all() {
  local d
  for d in ${RELEASE_LOCKS[@]+"${RELEASE_LOCKS[@]}"}; do rm -rf "$d"; done
  RELEASE_LOCKS=()
}

trap release_unlock_all EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
