# Sourced by scripts/check.sh (card #556): temp_clean runs a command with
# its temp folder pointed at a new, empty one, and fails if the command
# leaves anything there.
#
# Windows can't delete an open file, and tempfile's TempDir ignores the
# failure, so a test that drops its TempDir while something still holds a
# file in it leaks a folder on the PC and passes everywhere else. By Nov 5
# that had put 175,000 folders in %TEMP%.
#
# TMPDIR covers a build as well as the tests, and rustc and the linker can
# leave their own temp files (.tmpWQYphzlibclang_rt.osx.a on the Mac, card
# #607). So check.sh builds first with `cargo test --no-run` and guards
# only the run. Don't skip names like .tmpXXXXXX instead: that is also what
# a leaked TempDir is called.
#
# The folder is outside the repo on purpose: tests that make git repos in
# temp would otherwise find this one above them.

temp_clean() {
  tc_dir=$(mktemp -d /tmp/irori-temp.XXXXXX)
  tc_native=$tc_dir
  if command -v cygpath >/dev/null 2>&1; then
    tc_native=$(cygpath -m "$tc_dir")
  fi
  tc_status=0
  TMP="$tc_native" TEMP="$tc_native" TMPDIR="$tc_native" "$@" || tc_status=$?
  tc_left=$(ls -A "$tc_dir" | wc -l | tr -d ' ')
  if [ "$tc_left" != 0 ]; then
    echo "temp: \`$*\` left $tc_left in its temp folder:" >&2
    (cd "$tc_dir" && find . -maxdepth 2 | sed 's/^/  /' | head -n 30) >&2
    echo "temp: drop whatever holds these files before the TempDir: declare it" >&2
    echo "temp: last in the struct, and close stores that tasks still hold." >&2
    [ "$tc_status" != 0 ] || tc_status=1
  fi
  rm -rf "$tc_dir"
  return "$tc_status"
}
