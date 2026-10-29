// Package names picks the friendly name Irori gives a Claude session that
// nobody has named (card #21): a short word from a fixed list, distinct
// from every live agent's name, so the Board says "heron" rather than a
// folder name plus a hash.
package names

import (
	"hash/fnv"
	"strconv"
	"strings"
)

// Words is the list names come from: short, lowercase, easy to say and to
// tell apart. Append only: a session's first choice is its sessionId's
// hash over this list, so reordering would move names on restart.
var Words = []string{
	"acorn", "alder", "amber", "anchor", "aspen", "aster", "badger", "basil",
	"beacon", "birch", "bison", "bramble", "brook", "cedar", "cinder", "clover",
	"comet", "coral", "cove", "crane", "cricket", "cypress", "dahlia", "delta",
	"dune", "eagle", "ember", "fennel", "fern", "finch", "fjord", "flint",
	"forest", "fox", "garnet", "gecko", "ginger", "glacier", "grove", "harbor",
	"hazel", "heron", "hickory", "holly", "ibis", "indigo", "iris", "ivy",
	"jade", "jasper", "juniper", "kestrel", "kite", "koala", "lagoon", "lark",
	"laurel", "lemur", "lichen", "lily", "linden", "lotus", "lynx", "magpie",
	"maple", "marble", "marten", "meadow", "mesa", "mink", "minnow", "moss",
	"nectar", "newt", "nimbus", "oak", "ocelot", "olive", "onyx", "orca",
	"osprey", "otter", "owl", "panda", "pebble", "pelican", "pepper", "pine",
	"plover", "poppy", "prairie", "puffin", "quail", "quartz", "raven", "reed",
	"ridge", "robin", "rowan", "saffron", "sage", "salmon", "sequoia", "shale",
	"sparrow", "spruce", "stork", "summit", "swift", "tansy", "tern", "thistle",
	"thyme", "tiger", "topaz", "tundra", "tulip", "umber", "valley", "vireo",
	"walnut", "willow", "wren", "yarrow", "yew", "zephyr", "zinnia",
}

// Pick is the name for seed (a session id): the word its hash lands on,
// else the next word on the list that taken refuses, wrapping round. When
// every word is taken it adds a number ("heron2"). taken is asked with the
// lowercase name; it should compare case-insensitively.
func Pick(seed string, taken func(name string) bool) string {
	h := fnv.New32a()
	_, _ = h.Write([]byte(seed))
	start := int(h.Sum32() % uint32(len(Words)))
	for n := 1; ; n++ {
		for i := range Words {
			name := Words[(start+i)%len(Words)]
			if n > 1 {
				name += strconv.Itoa(n)
			}
			if taken == nil || !taken(name) {
				return name
			}
		}
	}
}

// Fold is the form names are compared in.
func Fold(name string) string { return strings.ToLower(strings.TrimSpace(name)) }
