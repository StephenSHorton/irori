//go:build windows

package identity

import (
	"unsafe"

	"golang.org/x/sys/windows"
)

// DefaultKeystore is DPAPI on Windows: secrets are sealed to the signed-in
// user, so another account or machine can't unseal a copied data dir.
func DefaultKeystore(dataDir string) Keystore { return DPAPIKeystore{} }

// DPAPIKeystore seals with CryptProtectData (current user scope).
type DPAPIKeystore struct{}

var dpapiEntropy = []byte("irori/key/1")

func (DPAPIKeystore) Name() string { return "dpapi" }

func blobOf(b []byte) *windows.DataBlob {
	if len(b) == 0 {
		return &windows.DataBlob{}
	}
	return &windows.DataBlob{Size: uint32(len(b)), Data: &b[0]}
}

func takeBlob(out *windows.DataBlob) []byte {
	defer windows.LocalFree(windows.Handle(unsafe.Pointer(out.Data)))
	return append([]byte(nil), unsafe.Slice(out.Data, out.Size)...)
}

func (DPAPIKeystore) Seal(plain []byte) ([]byte, error) {
	var out windows.DataBlob
	name, _ := windows.UTF16PtrFromString("Irori key")
	if err := windows.CryptProtectData(blobOf(plain), name, blobOf(dpapiEntropy), 0, nil, windows.CRYPTPROTECT_UI_FORBIDDEN, &out); err != nil {
		return nil, err
	}
	return takeBlob(&out), nil
}

func (DPAPIKeystore) Unseal(blob []byte) ([]byte, error) {
	var out windows.DataBlob
	if err := windows.CryptUnprotectData(blobOf(blob), nil, blobOf(dpapiEntropy), 0, nil, windows.CRYPTPROTECT_UI_FORBIDDEN, &out); err != nil {
		return nil, err
	}
	return takeBlob(&out), nil
}
