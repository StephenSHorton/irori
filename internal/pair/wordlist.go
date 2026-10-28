package pair

// The PGP word list (Juola and Zimmermann, PGPfone, 1995), as hato-code has
// it (suzuri libs/transfer/crates/hato-code/src/wordlist.rs), lower-cased:
// two lists of 256 words, one per byte value. Even positions use the
// two-syllable list and odd positions the three-syllable one, so a listener
// catches a dropped, doubled or swapped word. MIT, via the PGPfone manual.

// evenWords are the two-syllable words, by byte value.
var evenWords = [256]string{
	"aardvark", "absurd", "accrue", "acme", "adrift", "adult", "afflict", "ahead", "aimless", "algol",
	"allow", "alone", "ammo", "ancient", "apple", "artist", "assume", "athens", "atlas", "aztec", "baboon",
	"backfield", "backward", "basalt", "beaming", "bedlamp", "beehive", "beeswax", "befriend", "belfast",
	"berserk", "billiard", "bison", "blackjack", "blockade", "blowtorch", "bluebird", "bombast", "bookshelf",
	"brackish", "breadline", "breakup", "brickyard", "briefcase", "burbank", "button", "buzzard", "cement",
	"chairlift", "chatter", "checkup", "chisel", "choking", "chopper", "christmas", "clamshell", "classic",
	"classroom", "cleanup", "clockwork", "cobra", "commence", "concert", "cowbell", "crackdown", "cranky",
	"crowfoot", "crucial", "crumpled", "crusade", "cubic", "deadbolt", "deckhand", "dogsled", "dosage",
	"dragnet", "drainage", "dreadful", "drifter", "dropper", "drumbeat", "drunken", "dupont", "dwelling",
	"eating", "edict", "egghead", "eightball", "endorse", "endow", "enlist", "erase", "escape", "exceed",
	"eyeglass", "eyetooth", "facial", "fallout", "flagpole", "flatfoot", "flytrap", "fracture", "fragile",
	"framework", "freedom", "frighten", "gazelle", "geiger", "glasgow", "glitter", "glucose", "goggles",
	"goldfish", "gremlin", "guidance", "hamlet", "highchair", "hockey", "hotdog", "indoors", "indulge",
	"inverse", "involve", "island", "janus", "jawbone", "keyboard", "kickoff", "kiwi", "klaxon", "lockup",
	"merit", "minnow", "miser", "mohawk", "mural", "music", "neptune", "newborn", "nightbird", "obtuse",
	"offload", "oilfield", "optic", "orca", "payday", "peachy", "pheasant", "physique", "playhouse", "pluto",
	"preclude", "prefer", "preshrunk", "printer", "profile", "prowler", "pupil", "puppy", "python", "quadrant",
	"quiver", "quota", "ragtime", "ratchet", "rebirth", "reform", "regain", "reindeer", "rematch", "repay",
	"retouch", "revenge", "reward", "rhythm", "ringbolt", "robust", "rocker", "ruffled", "sawdust", "scallion",
	"scenic", "scorecard", "scotland", "seabird", "select", "sentence", "shadow", "showgirl", "skullcap",
	"skydive", "slingshot", "slothful", "slowdown", "snapline", "snapshot", "snowcap", "snowslide", "solo",
	"spaniel", "spearhead", "spellbind", "spheroid", "spigot", "spindle", "spoilage", "spyglass", "stagehand",
	"stagnate", "stairway", "standard", "stapler", "steamship", "stepchild", "sterling", "stockman", "stopwatch",
	"stormy", "sugar", "surmount", "suspense", "swelter", "tactics", "talon", "tapeworm", "tempest", "tiger",
	"tissue", "tonic", "tracker", "transit", "trauma", "treadmill", "trojan", "trouble", "tumor", "tunnel",
	"tycoon", "umpire", "uncut", "unearth", "unwind", "uproot", "upset", "upshot", "vapor", "village", "virus",
	"vulcan", "waffle", "wallet", "watchword", "wayside", "willow", "woodlark", "zulu",
}

// oddWords are the three-syllable words, by byte value.
var oddWords = [256]string{
	"adroitness", "adviser", "aggregate", "alkali", "almighty", "amulet", "amusement", "antenna",
	"applicant", "apollo", "armistice", "article", "asteroid", "atlantic", "atmosphere", "autopsy", "babylon",
	"backwater", "barbecue", "belowground", "bifocals", "bodyguard", "borderline", "bottomless", "bradbury",
	"brazilian", "breakaway", "burlington", "businessman", "butterfat", "camelot", "candidate", "cannonball",
	"capricorn", "caravan", "caretaker", "celebrate", "cellulose", "certify", "chambermaid", "cherokee",
	"chicago", "clergyman", "coherence", "combustion", "commando", "company", "component", "concurrent",
	"confidence", "conformist", "congregate", "consensus", "consulting", "corporate", "corrosion", "councilman",
	"crossover", "cumbersome", "customer", "dakota", "decadence", "december", "decimal", "designing", "detector",
	"detergent", "determine", "dictator", "dinosaur", "direction", "disable", "disbelief", "disruptive",
	"distortion", "divisive", "document", "embezzle", "enchanting", "enrollment", "enterprise", "equation",
	"equipment", "escapade", "eskimo", "everyday", "examine", "existence", "exodus", "fascinate", "filament",
	"finicky", "forever", "fortitude", "frequency", "gadgetry", "galveston", "getaway", "glossary", "gossamer",
	"graduate", "gravity", "guitarist", "hamburger", "hamilton", "handiwork", "hazardous", "headwaters",
	"hemisphere", "hesitate", "hideaway", "holiness", "hurricane", "hydraulic", "impartial", "impetus",
	"inception", "indigo", "inertia", "infancy", "inferno", "informant", "insincere", "insurgent", "integrate",
	"intention", "inventive", "istanbul", "jamaica", "jupiter", "leprosy", "letterhead", "liberty", "maritime",
	"matchmaker", "maverick", "medusa", "megaton", "microscope", "microwave", "midsummer", "millionaire",
	"miracle", "misnomer", "molasses", "molecule", "montana", "monument", "mosquito", "narrative", "nebula",
	"newsletter", "norwegian", "october", "ohio", "onlooker", "opulent", "orlando", "outfielder", "pacific",
	"pandemic", "pandora", "paperweight", "paragon", "paragraph", "paramount", "passenger", "pedigree",
	"pegasus", "penetrate", "perceptive", "performance", "pharmacy", "phonetic", "photograph", "pioneer",
	"pocketful", "politeness", "positive", "potato", "processor", "prophecy", "provincial", "proximate",
	"puberty", "publisher", "pyramid", "quantity", "racketeer", "rebellion", "recipe", "recover", "repellent",
	"replica", "reproduce", "resistor", "responsive", "retraction", "retrieval", "retrospect", "revenue",
	"revival", "revolver", "sahara", "sandalwood", "sardonic", "saturday", "savagery", "scavenger", "sensation",
	"sociable", "souvenir", "specialist", "speculate", "stethoscope", "stupendous", "supportive", "surrender",
	"suspicious", "sympathy", "tambourine", "telephone", "therapist", "tobacco", "tolerance", "tomorrow",
	"torpedo", "tradition", "travesty", "trombonist", "truncated", "typewriter", "ultimate", "undaunted",
	"underfoot", "unicorn", "unify", "universe", "unravel", "upcoming", "vacancy", "vagabond", "versatile",
	"vertigo", "virginia", "visitor", "vocalist", "voyager", "warranty", "waterloo", "whimsical", "wichita",
	"wilmington", "wyoming", "yesteryear", "yucatan",
}
