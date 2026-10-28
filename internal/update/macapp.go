package update

import (
	"path/filepath"
	"strings"
)

// The pieces of the macOS updater that are plain path and string work live
// here, untagged, so their tests run on every OS. mac_darwin.go does the
// rest.

// macBundleID is Irori.app's CFBundleIdentifier (build/darwin/Info.plist).
// An update whose bundle says otherwise is refused.
const macBundleID = "com.stephenshorton.irori"

// appBundleRoot returns the .app bundle exe runs from
// (.../Irori.app/Contents/MacOS/irori gives .../Irori.app), or "".
func appBundleRoot(exe string) string {
	macos := filepath.Dir(exe)
	if filepath.Base(macos) != "MacOS" {
		return ""
	}
	contents := filepath.Dir(macos)
	if filepath.Base(contents) != "Contents" {
		return ""
	}
	app := filepath.Dir(contents)
	if !strings.HasSuffix(app, ".app") {
		return ""
	}
	return app
}

// translocated reports whether app runs from App Translocation's read-only
// copy, which macOS uses for a quarantined app opened where it was
// downloaded. An update there would be thrown away.
func translocated(app string) bool { return strings.Contains(app, "/AppTranslocation/") }

// sidePaths are where an update stages the new bundle (.Irori.app.new) and
// parks the old one during the swap (.Irori.app.old): hidden siblings of the
// live bundle, so both renames stay on one volume.
func sidePaths(app string) (next, old string) {
	dir, base := filepath.Dir(app), filepath.Base(app)
	return filepath.Join(dir, "."+base+".new"), filepath.Join(dir, "."+base+".old")
}

// numericVersion is v without a leading v, prerelease or build part: the
// X.Y.Z that release.yml stamps into CFBundleShortVersionString.
func numericVersion(v string) string {
	v = strings.TrimPrefix(v, "v")
	if i := strings.IndexAny(v, "-+"); i >= 0 {
		v = v[:i]
	}
	return v
}

// swapScript is the detached helper that finishes a macOS update:
//
//	sh -c swapScript irori-update <pid> <live .app> <staged .app> <relaunch 0|1> <log>
//
// It waits (up to two minutes) for the GUI process to exit, moves the live
// bundle aside, moves the staged one into its place (moving the old one back
// if that fails), deletes the old one, and reopens Irori when relaunch is 1,
// so "Restart now" never leaves Irori closed. Processes still running from
// the old bundle (`irori mcp`) keep the image they opened.
const swapScript = `pid=$1 live=$2 next=$3 relaunch=$4
exec >>"$5" 2>&1
old="${live%/*}/.${live##*/}.old"
echo "$(/bin/date): update: waiting for $pid"
n=0
while kill -0 "$pid" 2>/dev/null; do
	n=$((n+1))
	if [ "$n" -gt 600 ]; then
		echo "update: $pid still running after 2 minutes; leaving $live as it is"
		exit 1
	fi
	/bin/sleep 0.2
done
/bin/rm -rf "$old"
if /bin/mv "$live" "$old"; then
	if /bin/mv "$next" "$live"; then
		/bin/rm -rf "$old"
		echo "update: installed $live"
	else
		/bin/mv "$old" "$live"
		echo "update: could not move $next into place; kept the old Irori"
	fi
else
	echo "update: could not move $live aside; kept the old Irori"
fi
if [ "$relaunch" = 1 ]; then
	exec /usr/bin/open "$live"
fi
`
