# Sourced by scripts/check.sh (card #437): take one of this device's check
# slots before the build, so at most CHECK_SLOTS full runs build at once
# (30 at once on Nov 3 put the Mac's load at 60 and filled its disk). A
# third run waits its turn. The slot is an Irori lease (POST /v1/slots,
# crates/api/src/slots.rs) on check/<host>/slot-N, held by this run, so
# the Board's leases show who has them.
#
# The lease is short (SLOT_TTL) and a watcher renews it while this shell
# lives. Exit, a failure, Ctrl+C or TERM releases it at once (the trap); a
# kill -9 is seen by the watcher within a few seconds, which releases it;
# if both die, the lease ends by itself after SLOT_TTL.
#
# With Irori not running (no lockfile, no answer, no curl), or
# CHECK_SLOTS=0, it does nothing and check.sh runs as before.

CHECK_SLOTS="${CHECK_SLOTS:-2}"
SLOT_TTL=120
SLOT_RENEW=30

slot_log() { echo "check slot: $*" >&2; }

# The data dir, as app/src/datadir.rs finds it.
slot_home() {
  if [ -n "${IRORI_HOME:-}" ]; then
    echo "$IRORI_HOME"
  elif [ -n "${LOCALAPPDATA:-}" ]; then
    echo "$LOCALAPPDATA/irori"
  else
    echo "$HOME/.irori"
  fi
}

# A field of the one-line lockfile JSON (irori.json): "port" or "token".
slot_field() {
  sed -n "s/.*\"$1\": *\"\{0,1\}\([^\",}]*\).*/\1/p" "$2" | head -n 1
}

# POST $1 (path and query); prints the body, then the status on its own line.
slot_post() {
  curl -sS --max-time 5 -X POST -w '\n%{http_code}' \
    -H "Authorization: Bearer $slot_token" \
    "http://127.0.0.1:$slot_port$1" 2>/dev/null || printf '\n000'
}

slot_release() {
  if [ -n "${slot_watcher:-}" ]; then
    kill "$slot_watcher" 2>/dev/null || true
    slot_watcher=""
  fi
  if [ -n "${slot_name:-}" ]; then
    slot_post "/v1/slots/release?resource=$slot_res&run=$slot_run" >/dev/null || true
    slot_log "released $slot_name"
    slot_name=""
  fi
}

slot_take() {
  case "$CHECK_SLOTS" in
    '' | *[!0-9]*) slot_log "CHECK_SLOTS must be a number, not $CHECK_SLOTS"; return 0 ;;
    0) return 0 ;;
  esac
  command -v curl >/dev/null 2>&1 || { slot_log "no curl: running without a slot"; return 0; }
  lock="$(slot_home)/irori.json"
  [ -f "$lock" ] || { slot_log "Irori isn't running: running without a slot"; return 0; }
  slot_port=$(slot_field port "$lock")
  slot_token=$(slot_field token "$lock")
  if [ -z "$slot_port" ] || [ -z "$slot_token" ] ||
    ! curl -sf --max-time 3 "http://127.0.0.1:$slot_port/v1/health" >/dev/null 2>&1; then
    slot_log "Irori isn't answering: running without a slot"
    return 0
  fi
  host=$(hostname 2>/dev/null | sed 's/\..*//' | tr 'A-Z' 'a-z' | tr -c 'a-z0-9.\n_-' '-')
  slot_res="check/${host:-device}"
  slot_run=$(printf 'check-%s-%s' "$(basename "$PWD")" "$$" | tr 'A-Z' 'a-z' | tr -c 'a-z0-9._-' '-' | cut -c 1-64)
  said=""
  while :; do
    out=$(slot_post "/v1/slots?resource=$slot_res&slots=$CHECK_SLOTS&ttl=$SLOT_TTL&run=$slot_run")
    code=$(printf '%s\n' "$out" | tail -n 1)
    body=$(printf '%s\n' "$out" | sed '$d')
    case "$code" in
      200)
        slot_name=$(printf '%s' "$body" | sed -n 's/.*"resource": *"\([^"]*\)".*/\1/p')
        slot_log "took $slot_name as $slot_run"
        break
        ;;
      409)
        held=$(printf '%s' "$body" | sed -n 's/.*"held": *\[\([^]]*\)\].*/\1/p' | sed 's/","/, /g; s/"//g')
        if [ "$held" != "$said" ]; then
          echo "waiting for a check slot (held by $held)" >&2
          said="$held"
        fi
        sleep 10
        ;;
      404 | 405)
        slot_log "this Irori has no check slots yet (older version): running without a slot"
        return 0
        ;;
      *)
        slot_log "Irori answered $code ($body): running without a slot"
        return 0
        ;;
    esac
  done
  trap 'slot_release' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  # The watcher renews while this shell lives and releases once it is
  # gone (a kill -9 runs no trap).
  parent=$$
  (
    trap - EXIT INT TERM
    n=0
    while kill -0 "$parent" 2>/dev/null; do
      sleep 3
      n=$((n + 3))
      if [ "$n" -ge "$SLOT_RENEW" ]; then
        n=0
        slot_post "/v1/slots?resource=$slot_res&slots=$CHECK_SLOTS&ttl=$SLOT_TTL&run=$slot_run" >/dev/null
      fi
    done
    slot_post "/v1/slots/release?resource=$slot_res&run=$slot_run" >/dev/null
  ) </dev/null >/dev/null 2>&1 &
  slot_watcher=$!
}

slot_take
