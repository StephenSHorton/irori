package blob

import "os"

// MarkDownloaded marks a file written out of the store as from another
// computer: a Zone.Identifier stream with the Internet zone (Mark of the
// Web), so SmartScreen, Office Protected View and the like treat it as a
// download.
func MarkDownloaded(path string) error {
	return os.WriteFile(path+":Zone.Identifier", []byte("[ZoneTransfer]\r\nZoneId=3\r\n"), 0o600)
}
