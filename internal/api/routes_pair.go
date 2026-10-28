package api

// Pairing routes (v0.4 §3, §6.8): the JOINING side only, so `irori pair
// join` can drive the running Core instead of starting its own. CONTRACT
// for the api and CLI: paths, statuses and JSON shapes below are binding.
// All routes keep v0.1's rules (bearer token, Host check, no Origin). The
// code, the server address and the key travel only in POST bodies, never
// in a query, and no answer or error text repeats the code or the key. The
// join's status does echo the server address back, normalized, as "via":
// it isn't a secret (A's Add screen shows it under the code).
//
//	POST /v1/pair/join   {"code": "...", "server": "<host:port, optional>"} -> pair.Status
//	     400 a bad code or server address; 409 a pairing is already open, or
//	     this device is already one of a person's devices (pair.ErrInMesh).
//	GET  /v1/pair/status -> pair.Status, the joining side's view: while an
//	     Add window is open (in the app) only {mode:"add", state} is shown,
//	     never its code or key.
//	POST /v1/pair/key    {"key": "..."} -> pair.Status
//	     422 the key isn't the one the other device's handshake proved (the
//	     miss counts; the second closes the window); 409 nothing waits for a
//	     key.
//	POST /v1/pair/cancel -> pair.Status; closes a join only, never an Add.
//
// 503 when the backend has no pairing (Pairing() absent or nil).
//
// There is NO route that opens a pairing window (Add), types the words on
// the adding side, or removes a device, and none may be added: Add hands
// the new device the person secret and every row, so it stays in the
// desktop app's Settings → Devices (§3).
//
// The gate is the bearer token alone, deliberately. The D10 person secret
// lives only in the GUI's memory, so a terminal can't hold it. The token
// does let any agent that reads irori.json run a join on its own: with a
// code and key from a device of its own, it could pull this device into
// that device's person. The impact runs both ways: this device's log
// replicates there, and that person's devices can then write user rows
// this device takes as the person's, which wake and steer its agent
// sessions (§6.3, §8). It also happens while the app runs, with nothing
// shown unless Settings → Devices is open. A token holder can likewise
// cancel, or fail with two wrong keys, a join the person started in the
// app, or take the one pairing slot first, and the status route shows that
// join's words and peer: a denial and a view, never a pairing, since the
// words still go into the other device and its key still gates this one.
// That adds no capability a shell doesn't already have: the same agent
// can quit Irori and pipe the code and key into `irori pair join` (it
// reads stdin when that isn't a terminal), or read the log from the data
// dir; only the quitting, which showed, is gone. The bounds
// that matter hold here too: a device that is already in a person can't
// join another (pair.ErrInMesh), and nothing on this API can add a device
// to the person, so an agent can't pull anyone out of it. A harder gate
// would need to know which process is on the other end of the loopback
// socket, which isn't worth it for no new capability.

// Pairing paths.
const (
	pathPairJoin   = "/v1/pair/join"
	pathPairStatus = "/v1/pair/status"
	pathPairKey    = "/v1/pair/key"
	pathPairCancel = "/v1/pair/cancel"
)

// MaxPairField caps the code, server and key a pairing body may carry.
const MaxPairField = 200
