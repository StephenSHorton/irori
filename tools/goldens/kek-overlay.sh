# Sourced by verify-live.sh and replay-live.sh, after $goirori, $work and
# native() are set.
#
# On macOS the Go Core's keystore names its Keychain item after the data
# dir's path, so it cannot open a copy. With IRORI_DEV_KEK_DIR set (as for
# Rust's dev builds, docs/m1-mac.md §2.2), kek_overlay writes
# $work/keychain.go, Go's own keychain.go whose kekAccounts names the item
# from that dir instead, and prints the overlay Replace entry for it, with
# a leading comma. Elsewhere it prints nothing. The Go repo stays untouched.
kek_overlay() {
  if [ -z "${IRORI_DEV_KEK_DIR:-}" ] || [ "$(uname)" != Darwin ]; then
    return 0
  fi
  src="$goirori/internal/identity/keychain.go"
  if ! awk '{ print }
    $0 == "\tcanon = filepath.Clean(canon)" {
      print "\tif d := os.Getenv(\"IRORI_DEV_KEK_DIR\"); d != \"\" {"
      print "\t\tcanon = filepath.Clean(d)"
      print "\t}"
      n++
    }
    END { exit n != 1 }' "$src" >"$work/keychain.go"; then
    echo "kek-overlay: Go's kekAccounts changed; update tools/goldens/kek-overlay.sh" >&2
    return 1
  fi
  printf ',"%s":"%s"' "$(native "$src")" "$(native "$work/keychain.go")"
}
