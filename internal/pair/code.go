// Package pair is v0.4's pairing flow (design/v0.4-devices.md §3): a device
// already in the mesh ("A", Add) shows a one-use code; the new device ("B",
// Join) enters it; both meet in a pairing room derived from the code, prove
// they hold it, and derive five SAS words over both device keys and the DTLS
// fingerprints of their one connection. B shows the words, the person types
// them into A, and only then does A certify B and hand it the person.
//
// Pairing is GUI-only (and the CLI that starts its own Core): nothing here is
// on the loopback API, so an agent with a shell can't pair a device.
package pair

import (
	"crypto/ed25519"
	"crypto/hkdf"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"strings"

	"golang.org/x/crypto/scrypt"

	"github.com/StephenSHorton/irori/internal/mesh"
)

// App is the pairing room's app name: the scrypt salt and topic prefix
// (go-room's SecretFromCode("irori-pair-v1", code), §4.2).
const App = "irori-pair-v1"

// QRPrefix starts the text a pairing QR code holds: "irori-pair:1:<code>",
// so a later format can be told apart (§3 step 1).
const QRPrefix = "irori-pair:1:"

// The room code (go-room's NewCode and NormalizeCode, written out here so
// only roomnet imports go-room; roomnet's tests check they agree): 16
// symbols of an alphabet with no 0/O or 1/I/L, about 79 bits.
const (
	codeAlphabet = "23456789ABCDEFGHJKMNPQRSTUVWXYZ"
	codeLen      = 16
)

// ErrBadCode is a string that isn't a pairing code.
var ErrBadCode = errors.New("pair: that isn't a pairing code (16 letters and numbers, like ABCD-EFGH-JKMN-PQRS)")

// NewCode returns a fresh code, XXXX-XXXX-XXXX-XXXX.
func NewCode() string {
	out := make([]byte, 0, codeLen)
	b := make([]byte, codeLen)
	for len(out) < codeLen {
		if _, err := rand.Read(b); err != nil {
			panic(err) // crypto/rand never fails on supported platforms
		}
		for _, v := range b {
			// Rejection sampling keeps every symbol equally likely.
			if v < byte(256/len(codeAlphabet)*len(codeAlphabet)) && len(out) < codeLen {
				out = append(out, codeAlphabet[int(v)%len(codeAlphabet)])
			}
		}
	}
	return formatCode(string(out))
}

func formatCode(raw string) string {
	var sb strings.Builder
	for i := 0; i < len(raw); i++ {
		if i > 0 && i%4 == 0 {
			sb.WriteByte('-')
		}
		sb.WriteByte(raw[i])
	}
	return sb.String()
}

// NormalizeCode accepts a code typed or pasted any way (any case, spaces,
// with or without dashes), or a scanned QR's "irori-pair:1:<code>", and
// returns its canonical form.
func NormalizeCode(in string) (string, bool) {
	in = strings.TrimSpace(in)
	if rest, ok := strings.CutPrefix(in, QRPrefix); ok {
		in = rest
	} else if strings.HasPrefix(in, "irori-pair:") {
		return "", false // a later QR format this build can't read
	}
	var raw strings.Builder
	for _, r := range strings.ToUpper(in) {
		switch {
		case r == '-' || r == ' ' || r == '\t':
			continue
		case r < 128 && strings.IndexByte(codeAlphabet, byte(r)) >= 0:
			raw.WriteRune(r)
		default:
			return "", false
		}
	}
	if raw.Len() != codeLen {
		return "", false
	}
	return formatCode(raw.String()), true
}

// QR is the text of code's QR code.
func QR(code string) string { return QRPrefix + code }

// RoomSecret is the pairing room of code: scrypt(code, salt = App), the
// first 32 bytes the sealing key and the topic App + "/" + hex(the next 16).
func RoomSecret(code string) (mesh.Secret, error) {
	canon, ok := NormalizeCode(code)
	if !ok {
		return mesh.Secret{}, ErrBadCode
	}
	dk, err := scrypt.Key([]byte(canon), []byte(App), 1<<15, 8, 1, 48)
	if err != nil {
		return mesh.Secret{}, err
	}
	var s mesh.Secret
	copy(s.Key[:], dk[:32])
	s.Topic = App + "/" + hex.EncodeToString(dk[32:])
	return s, nil
}

// Info is what the proof and the SAS cover (§3 step 3): both device keys,
// both DTLS fingerprints as the transport saw them, and both device ids,
// always in A-then-B order, each length-prefixed.
func Info(aPub, bPub ed25519.PublicKey, fpA, fpB, aID, bID string) []byte {
	var out []byte
	for _, f := range [][]byte{aPub, bPub, []byte(fpA), []byte(fpB), []byte(aID), []byte(bID)} {
		out = binary.BigEndian.AppendUint32(out, uint32(len(f)))
		out = append(out, f...)
	}
	return out
}

// SASSalt is the SAS's HKDF salt.
const SASSalt = "irori/pair/1"

// SASWords is the number of words the person reads and types.
const SASWords = 5

// SAS derives the words: HKDF-SHA256(ikm = the room key, salt =
// "irori/pair/1", info) → 5 bytes, each a PGP word (even list at even
// positions, odd list at odd ones).
func SAS(roomKey [32]byte, info []byte) []string {
	b, err := hkdf.Key(sha256.New, roomKey[:], []byte(SASSalt), string(info), SASWords)
	if err != nil {
		panic(err) // 5 bytes always fits
	}
	words := make([]string, len(b))
	for i, v := range b {
		if i%2 == 0 {
			words[i] = evenWords[v]
		} else {
			words[i] = oddWords[v]
		}
	}
	return words
}

// Proof is side's proof that it holds the code: HMAC-SHA256 under the room
// key over the side ('A' or 'B') and info. The side byte keeps one side's
// proof from being reflected back as the other's.
func Proof(roomKey [32]byte, side byte, info []byte) []byte {
	m := hmac.New(sha256.New, roomKey[:])
	m.Write([]byte{side})
	m.Write(info)
	return m.Sum(nil)
}

// CheckProof compares a received proof in constant time.
func CheckProof(roomKey [32]byte, side byte, info, got []byte) bool {
	return hmac.Equal(Proof(roomKey, side, info), got)
}

// ParseWords splits what the person typed into words: any case, separated
// by spaces, commas or dashes.
func ParseWords(typed string) []string {
	return strings.FieldsFunc(strings.ToLower(typed), func(r rune) bool {
		return r == ' ' || r == ',' || r == '-' || r == '\t' || r == '\n' || r == '\r' || r == '.'
	})
}

// WordsMatch reports whether typed names exactly want, in order.
func WordsMatch(want []string, typed string) bool {
	got := ParseWords(typed)
	if len(got) != len(want) {
		return false
	}
	ok := 1
	for i := range want {
		ok &= hmacEq(got[i], want[i])
	}
	return ok == 1
}

func hmacEq(a, b string) int {
	if hmac.Equal([]byte(a), []byte(b)) {
		return 1
	}
	return 0
}

// Fingerprint is the short form of a device key the add side shows above
// the words box: the first 8 bytes of its sha256, hex, in groups of four.
func Fingerprint(pub ed25519.PublicKey) string {
	sum := sha256.Sum256(pub)
	h := hex.EncodeToString(sum[:8])
	return h[0:4] + " " + h[4:8] + " " + h[8:12] + " " + h[12:16]
}
