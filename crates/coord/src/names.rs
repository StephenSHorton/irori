//! The friendly name Irori gives a Claude session nobody named (card #21;
//! I:internal/names, coord.md §3.13): a short word from a fixed list,
//! distinct from every live agent's name.

use gocompat::fnv::fnv32a;

/// The list names come from (card #224: about a thousand, so a name comes
/// back weeks later, not the next day): every theme's words, in
/// [`THEMES`] order. Every word is 3-8 lowercase letters, and none is the
/// start of another, so an @mention of one never reads as another. Append
/// only, a new theme at the end: a new session's first choice is its
/// session id's hash over this list. A session keeps the name it was given
/// (names.json), so a longer list never renames a running agent.
pub const WORDS: &[&str] = &ALL;

/// One theme of words (card #702): Settings can turn each off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// What the setting stores.
    pub id: &'static str,
    /// What Settings shows.
    pub label: &'static str,
    pub words: &'static [&'static str],
    /// A few of its best-known words, for Settings.
    pub sample: &'static [&'static str],
}

/// The themes, in the order their words sit in [`WORDS`]. Append only.
pub const THEMES: &[Theme] = &[
    Theme {
        id: "nature",
        label: "Nature",
        words: NATURE,
        sample: &["kestrel", "cedar", "quartz", "heron"],
    },
    Theme {
        id: "produce",
        label: "Produce and things",
        words: PRODUCE,
        sample: &["onion", "tomato", "kettle", "mitten"],
    },
    Theme {
        id: "movies",
        label: "Movie characters",
        words: MOVIES,
        sample: &["ripley", "marty", "dorothy", "atticus"],
    },
    Theme {
        id: "anime",
        label: "Anime characters",
        words: ANIME,
        sample: &["chihiro", "spike", "levi", "sakura"],
    },
    Theme {
        id: "presidents",
        label: "US presidents",
        words: PRESIDENTS,
        sample: &["lincoln", "adams", "polk", "taft"],
    },
    Theme {
        id: "myth",
        label: "Mythology",
        words: MYTH,
        sample: &["zeus", "athena", "odin", "anubis"],
    },
];

/// The theme with this id; `None` for an id this build doesn't know.
pub fn theme(id: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|t| t.id == id)
}

/// The words new names come from when the person turned the themes in
/// `off` off (card #702): the other themes' words, in [`WORDS`] order.
/// `None` means all of [`WORDS`]: no theme this build knows is off, or
/// every one is (then names come from the full list, never from none).
pub fn words_on(off: &[String]) -> Option<Vec<&'static str>> {
    let is_off = |t: &Theme| off.iter().any(|o| o == t.id);
    let on: Vec<&Theme> = THEMES.iter().filter(|t| !is_off(t)).collect();
    if on.is_empty() || on.len() == THEMES.len() {
        return None;
    }
    Some(on.iter().flat_map(|t| t.words.iter().copied()).collect())
}

const N_WORDS: usize = {
    let mut n = 0;
    let mut i = 0;
    while i < THEMES.len() {
        n += THEMES[i].words.len();
        i += 1;
    }
    n
};

static ALL: [&str; N_WORDS] = {
    let mut out = [""; N_WORDS];
    let (mut k, mut i) = (0, 0);
    while i < THEMES.len() {
        let w = THEMES[i].words;
        let mut j = 0;
        while j < w.len() {
            out[k] = w[j];
            k += 1;
            j += 1;
        }
        i += 1;
    }
    out
};

/// Card #224's words: nature, gems, tools, instruments and colours.
const NATURE: &[&str] = &[
    "acorn", "alder", "amber", "anchor", "aspen", "aster", "badger", "basil", "beacon", "birch",
    "bison", "bramble", "brook", "cedar", "cinder", "clover", "comet", "coral", "cove", "crane",
    "cricket", "cypress", "dahlia", "delta", "dune", "eagle", "ember", "fennel", "fern", "finch",
    "fjord", "flint", "forest", "fox", "garnet", "gecko", "ginger", "glacier", "grove", "harbor",
    "hazel", "heron", "hickory", "holly", "ibis", "indigo", "iris", "ivy", "jade", "jasper",
    "juniper", "kestrel", "kite", "koala", "lagoon", "lark", "laurel", "lemur", "lichen", "lily",
    "linden", "lotus", "lynx", "magpie", "maple", "marble", "marten", "meadow", "mesa", "mink",
    "minnow", "moss", "nectar", "newt", "nimbus", "oak", "ocelot", "olive", "onyx", "orca",
    "osprey", "otter", "owl", "panda", "pebble", "pelican", "pepper", "pine", "plover", "poppy",
    "prairie", "puffin", "quail", "quartz", "raven", "reed", "ridge", "robin", "rowan", "saffron",
    "sage", "salmon", "sequoia", "shale", "sparrow", "spruce", "stork", "summit", "swift", "tansy",
    "tern", "thistle", "thyme", "tiger", "topaz", "tundra", "tulip", "umber", "valley", "vireo",
    "walnut", "willow", "wren", "yarrow", "yew", "zephyr", "zinnia", "beech", "fir", "larch",
    "poplar", "sumac", "acacia", "baobab", "banyan", "balsa", "cassia", "catalpa", "mahogany",
    "mangrove", "myrtle", "palmetto", "quince", "redwood", "teak", "tamarind", "tupelo", "yucca",
    "ironwood", "boxwood", "buckeye", "chestnut", "dogwood", "hemlock", "locust", "mulberry",
    "hawthorn", "sycamore", "tamarack", "wisteria", "magnolia", "apricot", "pawpaw", "medlar",
    "loquat", "guava", "lychee", "mango", "papaya", "kumquat", "citron", "pomelo", "carob",
    "kapok", "eucalypt", "kauri", "rimu", "totara", "manuka", "kowhai", "azalea", "begonia",
    "camellia", "crocus", "freesia", "gardenia", "geranium", "hyacinth", "jasmine", "lavender",
    "lilac", "marigold", "orchid", "peony", "petunia", "primrose", "snowdrop", "bluebell",
    "cosmos", "heather", "hibiscus", "lupin", "phlox", "sorrel", "trillium", "verbena", "anemone",
    "arnica", "borage", "calla", "clematis", "daphne", "fuchsia", "gentian", "gorse", "hosta",
    "jonquil", "kalmia", "lantana", "lobelia", "lunaria", "mimosa", "nigella", "oleander",
    "oxalis", "protea", "salvia", "scilla", "sedum", "statice", "tuberose", "valerian", "vetch",
    "viola", "xenia", "allium", "alyssum", "astilbe", "bergenia", "briar", "canna", "celosia",
    "clary", "cyclamen", "dianthus", "erica", "gerbera", "godetia", "ixia", "liatris", "malva",
    "nerine", "nemesia", "orris", "pansy", "plumbago", "quamash", "scabious", "thrift", "tithonia",
    "ursinia", "vinca", "weigela", "anise", "caraway", "cardamom", "chervil", "chicory", "chive",
    "cilantro", "cumin", "dill", "garlic", "hyssop", "lovage", "marjoram", "nutmeg", "oregano",
    "paprika", "parsley", "rosemary", "savory", "sesame", "tarragon", "turmeric", "wasabi",
    "angelica", "bayleaf", "burdock", "catnip", "comfrey", "costmary", "dittany", "galangal",
    "licorice", "mugwort", "nettle", "perilla", "purslane", "shiso", "stevia", "woodruff",
    "zaatar", "avocet", "bittern", "bluejay", "bobolink", "bunting", "buzzard", "canary",
    "cardinal", "condor", "coot", "cowbird", "cuckoo", "curlew", "dipper", "dove", "dunlin",
    "egret", "eider", "emu", "falcon", "flamingo", "gannet", "godwit", "goose", "grackle", "grebe",
    "grouse", "gull", "harrier", "hawk", "hoopoe", "hornbill", "jackdaw", "junco", "kakapo", "kea",
    "kingbird", "lapwing", "linnet", "loon", "lorikeet", "macaw", "mallard", "merlin", "moorhen",
    "mynah", "nuthatch", "oriole", "ostrich", "ouzel", "parakeet", "peacock", "penguin", "petrel",
    "pheasant", "phoebe", "pigeon", "pipit", "pochard", "quetzal", "redpoll", "redstart", "shrike",
    "siskin", "skua", "skylark", "snipe", "starling", "sunbird", "swallow", "swan", "tanager",
    "thrush", "titmouse", "toucan", "towhee", "turaco", "veery", "wagtail", "warbler", "waxwing",
    "weaver", "whimbrel", "wigeon", "woodcock", "bulbul", "drongo", "fulmar", "gadwall", "jacana",
    "kakariki", "motmot", "oxpecker", "pewee", "pitta", "potoo", "prion", "scaup", "scoter",
    "smew", "tui", "tody", "trogon", "whydah", "aardvark", "alpaca", "antelope", "axolotl",
    "baboon", "beaver", "bobcat", "bongo", "capybara", "caribou", "chamois", "cheetah", "chipmunk",
    "civet", "coati", "cougar", "coyote", "dingo", "dormouse", "duiker", "eland", "elk", "ermine",
    "ferret", "fennec", "gazelle", "gerbil", "gibbon", "giraffe", "gnu", "gopher", "gorilla",
    "hamster", "hedgehog", "hippo", "ibex", "impala", "jackal", "jaguar", "jerboa", "kangaroo",
    "kinkajou", "kudu", "lemming", "leopard", "llama", "loris", "manatee", "mandrill", "marmot",
    "meerkat", "mongoose", "moose", "muskox", "narwhal", "numbat", "okapi", "opossum", "oryx",
    "pangolin", "panther", "peccary", "pika", "platypus", "possum", "puma", "quokka", "quoll",
    "raccoon", "reindeer", "rhino", "sable", "serval", "shrew", "sloth", "stoat", "tapir",
    "tarsier", "tenrec", "vicuna", "vole", "wallaby", "walrus", "warthog", "weasel", "wildcat",
    "wombat", "yak", "zebra", "zebu", "caracal", "dhole", "fossa", "galago", "genet", "grison",
    "hutia", "margay", "nilgai", "nyala", "oncilla", "pudu", "saiga", "takin", "tayra", "urial",
    "wapiti", "zorilla", "abalone", "anchovy", "barnacle", "beluga", "bonito", "catfish", "clam",
    "cockle", "cod", "conch", "dolphin", "dory", "eel", "flounder", "grouper", "haddock", "hake",
    "halibut", "herring", "kelp", "krill", "limpet", "lobster", "mackerel", "marlin", "mollusk",
    "monkfish", "mussel", "nautilus", "octopus", "oyster", "piranha", "plaice", "pollock", "prawn",
    "remora", "sardine", "scallop", "shark", "shrimp", "snapper", "sponge", "squid", "starfish",
    "sturgeon", "sunfish", "tarpon", "tilapia", "trout", "tuna", "turbot", "urchin", "wahoo",
    "whelk", "wrasse", "zander", "brill", "gudgeon", "guppy", "loach", "mullet", "nori", "orfe",
    "tench", "walleye", "bluefin", "cobia", "grunion", "opah", "pompano", "beetle", "cicada",
    "hornet", "katydid", "ladybug", "mantis", "mayfly", "midge", "moth", "scarab", "wasp",
    "caddis", "lacewing", "sawfly", "stonefly", "earwig", "adder", "anole", "boa", "caiman",
    "gharial", "iguana", "krait", "mamba", "skink", "tortoise", "tuatara", "agama", "agate",
    "amethyst", "azurite", "basalt", "beryl", "bismuth", "calcite", "citrine", "cobalt",
    "corundum", "diamond", "dolomite", "emerald", "feldspar", "fluorite", "galena", "gneiss",
    "granite", "graphite", "gypsum", "hematite", "iolite", "jacinth", "kyanite", "lapis", "mica",
    "obsidian", "opal", "peridot", "platinum", "pumice", "pyrite", "ruby", "rutile", "sapphire",
    "schist", "spinel", "sulfur", "talc", "titanium", "zircon", "chert", "cinnabar", "diorite",
    "gabbro", "mercury", "nephrite", "pewter", "porphyry", "scoria", "tektite", "tufa", "zeolite",
    "argon", "xenon", "krypton", "boron", "cerium", "erbium", "indium", "iridium", "osmium",
    "rhodium", "yttrium", "aurora", "blizzard", "breeze", "cirrus", "cyclone", "drizzle",
    "monsoon", "rainbow", "sleet", "squall", "sunbeam", "sunrise", "sunset", "thunder", "tornado",
    "twilight", "typhoon", "chinook", "mistral", "sirocco", "foehn", "halo", "stratus", "cumulus",
    "sundog", "vapor", "rime", "graupel", "flurry", "downpour", "antares", "aquila", "arcturus",
    "aries", "auriga", "bootes", "callisto", "capella", "carina", "castor", "cepheus", "cetus",
    "corvus", "cygnus", "deneb", "draco", "europa", "galaxy", "gemini", "halley", "hydra",
    "jupiter", "kepler", "libra", "lyra", "meteor", "mira", "nebula", "neptune", "nova", "orion",
    "pavo", "pegasus", "perseus", "phoenix", "pluto", "polaris", "pollux", "procyon", "pulsar",
    "quasar", "regulus", "rigel", "saturn", "sirius", "spica", "triton", "uranus", "vega", "virgo",
    "zenith", "equinox", "solstice", "altair", "mizar", "alcor", "ariel", "oberon", "phobos",
    "deimos", "dione", "tethys", "umbriel", "larissa", "naiad", "thalassa", "despina", "proteus",
    "nereid", "arroyo", "atoll", "bayou", "bluff", "boulder", "butte", "canyon", "cavern", "cliff",
    "crag", "crater", "desert", "estuary", "ford", "gorge", "glade", "gulch", "highland", "hollow",
    "inlet", "island", "isthmus", "knoll", "ledge", "moraine", "oasis", "outcrop", "pampas",
    "plateau", "rapids", "ravine", "reef", "savanna", "scree", "shoal", "steppe", "strait",
    "stream", "tarn", "terrace", "thicket", "volcano", "wetland", "geyser", "grotto", "cairn",
    "loch", "esker", "drumlin", "kame", "karst", "sinkhole", "caldera", "cirque", "couloir",
    "scarp", "talus", "wadi", "playa", "sabkha", "avocado", "bagel", "banana", "biscuit",
    "brioche", "caramel", "cashew", "cocoa", "coconut", "crepe", "crumpet", "currant", "custard",
    "dumpling", "eggplant", "fudge", "granola", "grape", "kale", "leek", "lentil", "macaron",
    "melon", "millet", "mochi", "muffin", "nougat", "okra", "pancake", "parsnip", "peanut",
    "popcorn", "pretzel", "prune", "pumpkin", "quinoa", "raisin", "ramen", "risotto", "scone",
    "sorbet", "spinach", "sprout", "strudel", "sushi", "toffee", "tofu", "truffle", "turnip",
    "waffle", "yam", "yogurt", "cobbler", "crumble", "dango", "gnocchi", "kimchi", "latte",
    "matcha", "miso", "panko", "pesto", "polenta", "ravioli", "tahini", "tamale", "udon", "wonton",
    "gelato", "biscotti", "brownie", "cupcake", "donut", "eclair", "flan", "mousse", "praline",
    "torte", "alps", "andes", "avalon", "azores", "borneo", "danube", "everest", "ganges", "nile",
    "sahara", "tahiti", "yukon", "canoe", "chime", "compass", "crystal", "cymbal", "easel",
    "fable", "feather", "fiddle", "flute", "forge", "fountain", "gable", "gadget", "gazebo",
    "glider", "goblet", "harp", "helm", "hammock", "kayak", "lantern", "lattice", "locket", "loom",
    "lute", "mantle", "mosaic", "oar", "orbit", "paddle", "pagoda", "palette", "parasol",
    "pendant", "piano", "prism", "pulley", "quill", "quiver", "raft", "rudder", "sail", "satchel",
    "scroll", "sextant", "sickle", "spindle", "spire", "spur", "stencil", "tambour", "teapot",
    "thimble", "tiller", "timber", "trellis", "trumpet", "tuba", "turret", "vase", "velvet",
    "violin", "windmill", "yacht", "yoke", "zither", "banjo", "cello", "cornet", "dulcimer",
    "gong", "lyre", "mandolin", "marimba", "oboe", "ocarina", "piccolo", "sitar", "tabla",
    "ukulele", "kazoo", "azure", "cerise", "crimson", "ecru", "fawn", "henna", "ochre", "puce",
    "russet", "sepia", "sienna", "taupe", "viridian",
];

/// Card #702: produce, pantry foods and everyday things; words that
/// read as code or app terms (thread, fork, string) are left out.
const PRODUCE: &[&str] = &[
    "abacus", "allspice", "almond", "apple", "apron", "arugula", "balloon", "barley", "barrel",
    "basket", "bauble", "bean", "berry", "blanket", "blender", "blini", "bobbin", "bokchoy",
    "bolt", "bonbon", "bookend", "bottle", "bowl", "bread", "brooch", "broom", "brush", "bubble",
    "buckle", "bulgur", "burrito", "cabbage", "candle", "canvas", "caper", "carrot", "cassava",
    "celery", "cereal", "chai", "chalk", "chapati", "checkers", "cheese", "cherry", "chess",
    "chickpea", "chili", "chowder", "churro", "ciabatta", "cider", "cinnamon", "clock", "coaster",
    "colander", "confetti", "conker", "cookie", "cork", "couscous", "cradle", "crayon", "crostini",
    "crouton", "cruet", "cucumber", "cushion", "daikon", "dice", "doily", "domino", "doorknob",
    "doughnut", "durian", "duvet", "endive", "envelope", "eraser", "espresso", "falafel", "fig",
    "flour", "focaccia", "freezer", "fridge", "funnel", "fusilli", "garland", "globe", "glove",
    "gourd", "grater", "gravy", "griddle", "gumdrop", "habanero", "hamper", "hinge", "honey",
    "hummus", "inkwell", "jalapeno", "jam", "jelly", "jicama", "jigsaw", "jug", "ketchup",
    "kettle", "keyring", "kiwi", "knight", "kohlrabi", "ladle", "lamp", "lasagna", "latch",
    "lemon", "lettuce", "lime", "lollipop", "longan", "magnet", "maize", "marzipan", "meringue",
    "mesclun", "mint", "mitten", "mixer", "mocha", "mop", "mortar", "muesli", "mustard", "naan",
    "nacho", "napkin", "needle", "noodle", "oats", "onion", "orange", "oven", "padlock", "paella",
    "pakora", "pawn", "peach", "pecan", "peeler", "pencil", "penne", "pestle", "pickle", "pillow",
    "pimento", "pinata", "pinwheel", "pita", "pitcher", "placemat", "plantain", "platter",
    "pliers", "pocket", "pogo", "pompom", "porridge", "postcard", "potato", "pudding", "purse",
    "puzzle", "quilt", "radish", "rambutan", "ramekin", "rattle", "relish", "rhubarb", "ribbon",
    "rice", "romaine", "rook", "roti", "ruler", "rutabaga", "salsa", "samosa", "satsuma", "saucer",
    "scarf", "scissors", "scooter", "shaker", "shallot", "sherbet", "shovel", "sieve", "skates",
    "skillet", "sled", "slipper", "slushie", "smoothie", "soba", "sock", "soda", "soup", "spatula",
    "spool", "spoon", "sprinkle", "squash", "stamp", "stapler", "steamer", "stew", "sticker",
    "sugar", "sundae", "syrup", "taco", "tangelo", "tape", "taro", "tassel", "teacake", "teacup",
    "teaspoon", "tinsel", "tomato", "tongs", "tortilla", "tricycle", "trifle", "trivet", "trowel",
    "tureen", "twine", "umbrella", "vanilla", "vinegar", "wagon", "wallet", "wellie", "wheat",
    "whisk", "whistle", "wok", "yarn", "yoyo", "zipper", "zucchini",
];

/// Card #702: film characters, by the names people call them; names a
/// franchise coined and polices are left out.
const MOVIES: &[&str] = &[
    "adrian", "agnes", "aladdin", "alfred", "alice", "amadeus", "amelie", "annie", "astrid",
    "atticus", "bagheera", "baloo", "bambi", "bastian", "bella", "belle", "benhur", "bishop",
    "bourne", "brody", "buddy", "cameron", "carl", "clarice", "cooper", "dallas", "deckard",
    "dolly", "dorothy", "eames", "effie", "eliza", "ellie", "elliott", "elsa", "elwood", "emmett",
    "ferris", "forrest", "gatsby", "genie", "gigi", "gilda", "glinda", "gomez", "gustave",
    "hiccup", "higgins", "hooper", "iceman", "ilsa", "indy", "inigo", "jessie", "joel", "jules",
    "kane", "lara", "lawrence", "lebowski", "leon", "liesl", "lilo", "logan", "lois", "louise",
    "luke", "lydia", "mame", "margot", "marion", "marty", "mathilda", "mattie", "maude",
    "maverick", "maximus", "merida", "mia", "mikey", "mulan", "murph", "nala", "nemo", "neo",
    "neville", "norma", "olaf", "poe", "poppins", "rachael", "rapunzel", "remy", "rey", "rhett",
    "rick", "ringo", "ripley", "rocky", "rooster", "roxie", "roy", "rudy", "russell", "sabrina",
    "salieri", "sally", "sandy", "scarlett", "scout", "scrooge", "selina", "shane", "sloane",
    "sully", "sundance", "suzy", "tevye", "thelma", "thumper", "tiana", "timon", "tinman", "toto",
    "travis", "tuco", "velma", "vontrapp", "wallace", "wendy", "westley", "zhivago",
];

/// Card #702: anime and manga characters, romanised; names a franchise
/// coined and polices are left out.
const ANIME: &[&str] = &[
    "gohan", "zoro", "nami", "sanji", "chopper", "sasuke", "sakura", "kakashi", "itachi",
    "chihiro", "haku", "nausicaa", "ashitaka", "sophie", "howl", "satsuki", "mei", "pazu",
    "sheeta", "arrietty", "spike", "jet", "faye", "ein", "levi", "eren", "mikasa", "armin",
    "hange", "ichigo", "rukia", "renji", "urahara", "edward", "alphonse", "winry", "riza",
    "shinji", "asuka", "kaworu", "misato", "usagi", "ami", "makoto", "minako", "tanjiro", "nezuko",
    "zenitsu", "inosuke", "giyu", "shinobu", "deku", "ochaco", "shoto", "saitama", "genos",
    "reigen", "yuji", "megumi", "nobara", "gojo", "maki", "denji", "anya", "loid", "yor", "himmel",
    "kenshin", "kaoru", "kagome", "sango", "shippo", "yusuke", "kurama", "hiei", "misty", "brock",
    "tetsuo", "akira", "motoko", "batou", "vash", "alucard", "suzaku", "asuna", "holo", "emilia",
    "subaru", "kazuma", "haruhi", "kyon", "nagato", "mugen", "jin", "fuu", "hitagi", "araragi",
    "okabe", "kurisu", "mayuri", "tohru", "yuki", "shoyo", "kageyama", "mitsuha", "hodaka",
    "suzume", "violet", "taiga", "kaguya", "chika", "miyuki", "bocchi", "nijika", "yotsuba",
    "chiyo", "osaka", "yui", "mio", "ritsu", "mugi", "azusa", "nanami", "tomoe", "ginko",
    "natsume", "nyanko", "madoka", "homura", "sayaka", "kyoko", "amuro", "char", "astro", "nobita",
    "shizuka", "conan", "heiji", "jigen", "goemon", "fujiko", "zenigata", "kenshiro", "jotaro",
    "josuke", "giorno", "joseph", "senku", "gintoki", "kagura", "sadaharu", "ryoma", "sena",
    "rukawa", "mitsui", "tsubasa", "nezumi", "shion", "hajime", "ippo", "yoh", "mononoke",
    "kodama", "yubaba", "kamaji", "moro", "eboshi", "nanoha",
];

/// Card #702: US presidents by surname, or a well-known short name when
/// the surname is over 8 letters (george, teddy, ike). Living presidents
/// are left out.
const PRESIDENTS: &[&str] = &[
    "george", "adams", "madison", "monroe", "jackson", "vanburen", "harrison", "tyler", "polk",
    "taylor", "fillmore", "pierce", "buchanan", "lincoln", "grant", "hayes", "garfield", "arthur",
    "mckinley", "teddy", "taft", "wilson", "harding", "coolidge", "hoover", "truman", "ike",
    "kennedy", "carter",
];

/// Card #702: gods, heroes and creatures of the world's myths; deities of
/// living major religions are left out.
const MYTH: &[&str] = &[
    "zeus", "athena", "odin", "thor", "loki", "anubis", "horus", "thoth", "raijin", "maui",
    "freya", "apollo", "artemis", "hermes", "ares", "hera", "poseidon", "hades", "demeter",
    "hestia", "dionysus", "gaia", "kronos", "rhea", "helios", "selene", "eos", "nyx", "nike",
    "hecate", "psyche", "atlas", "hercules", "theseus", "jason", "achilles", "odysseus", "orpheus",
    "eurydice", "medusa", "icarus", "daedalus", "ariadne", "hector", "ajax", "penelope", "calypso",
    "circe", "chiron", "minotaur", "cerberus", "sphinx", "griffin", "nemesis", "tyche", "morpheus",
    "hypnos", "juno", "minerva", "mars", "venus", "vulcan", "vesta", "diana", "ceres", "bacchus",
    "janus", "faunus", "flora", "romulus", "remus", "aeneas", "frigg", "baldur", "heimdall", "tyr",
    "freyr", "njord", "idun", "bragi", "sif", "skadi", "fenrir", "valkyrie", "mimir", "ymir",
    "sleipnir", "huginn", "muninn", "ratatosk", "osiris", "bastet", "sekhmet", "hathor", "ptah",
    "amun", "atum", "geb", "tefnut", "sobek", "khonsu", "maat", "nephthys", "khepri", "susanoo",
    "inari", "fujin", "izanagi", "izanami", "hachiman", "benten", "ebisu", "daikoku", "kitsune",
    "tanuki", "tengu", "kappa", "ryujin", "momotaro", "kintaro", "dagda", "lugh", "brigid",
    "morrigan", "finn", "oisin", "epona", "manannan", "aengus", "banshee", "selkie", "gwydion",
    "rhiannon", "taliesin", "enlil", "enki", "ishtar", "inanna", "marduk", "tiamat", "ninurta",
    "shamash", "tlaloc", "xolotl", "kukulkan", "itzamna", "chac", "tonatiuh", "ixchel", "inti",
    "perun", "veles", "mokosh", "svarog", "dazhbog", "stribog", "babayaga", "domovoi", "rusalka",
    "kupala", "tangaroa", "tane", "hina", "rangi", "kanaloa", "lono", "rongo", "tawhiri", "nuwa",
    "fuxi", "houyi", "wukong", "nezha", "pangu", "zhongkui", "qilin", "baku", "hotei", "jurojin",
    "helen", "leda", "danae", "ganymede", "nereus", "thetis", "galatea", "pandora", "atalanta",
    "hyperion", "oceanus", "themis", "clio", "thalia", "erato", "urania", "calliope", "euterpe",
    "pomona", "fortuna", "sigyn", "ullr", "vidar", "vali", "forseti", "norn", "skuld", "verdandi",
    "mani", "aegir", "jord", "sigurd", "beowulf", "brunhild", "gudrun", "surtr", "thrud", "magni",
    "bennu", "seshat", "wadjet", "nekhbet", "taweret", "aten", "imhotep", "neith",
];

/// The name for `seed` (a session id): the word its hash lands on, else the
/// next word along that `taken` refuses, wrapping round; when every word is
/// taken, the same probe with `2`, then `3`, ... appended. `taken` is asked
/// with the lowercase name.
pub fn pick(seed: &str, taken: Option<&dyn Fn(&str) -> bool>) -> String {
    let n_words = WORDS.len();
    let start = (fnv32a(seed.as_bytes()) % n_words as u32) as usize;
    let mut n = 1u64;
    loop {
        for i in 0..n_words {
            let mut name = WORDS[(start + i) % n_words].to_string();
            if n > 1 {
                name.push_str(&n.to_string());
            }
            match taken {
                Some(t) if t(&name) => {}
                _ => return name,
            }
        }
        n += 1;
    }
}

/// The first word for `seed` that `taken` doesn't refuse, probing the list
/// once from the word its hash lands on; `None` when every word is taken
/// (then [`pick`] goes on to numbered names).
pub fn pick_free(seed: &str, taken: &dyn Fn(&str) -> bool) -> Option<String> {
    pick_free_in(WORDS, seed, taken)
}

/// [`pick_free`] over `words` alone (one theme's, card #702): the hash
/// lands among them.
pub fn pick_free_in(words: &[&str], seed: &str, taken: &dyn Fn(&str) -> bool) -> Option<String> {
    let n_words = words.len();
    if n_words == 0 {
        return None;
    }
    let start = (fnv32a(seed.as_bytes()) % n_words as u32) as usize;
    (0..n_words)
        .map(|i| words[(start + i) % n_words])
        .find(|w| !taken(w))
        .map(str::to_string)
}

/// The form names are compared in: `strings.ToLower(strings.TrimSpace(name))`.
pub fn fold(name: &str) -> String {
    gocompat::text::to_lower_str(trim_space(name))
}

/// `strings.TrimSpace`.
pub(crate) fn trim_space(s: &str) -> &str {
    s.trim_matches(gocompat::text::is_space)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn words_are_many_distinct_short_and_prefix_free() {
        assert!(WORDS.len() >= 1000, "only {} words", WORDS.len());
        let mut seen = HashSet::new();
        for w in WORDS {
            assert!(seen.insert(*w), "{w} twice");
            assert!(
                (3..=8).contains(&w.len()) && w.bytes().all(|b| b.is_ascii_lowercase()),
                "{w}: not 3-8 lowercase letters"
            );
        }
        let mut sorted: Vec<&str> = WORDS.to_vec();
        sorted.sort_unstable();
        // In sorted order a word that starts another sits right before one
        // that starts with it.
        for pair in sorted.windows(2) {
            assert!(
                !pair[1].starts_with(pair[0]),
                "{} is the start of {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn themes_make_up_the_words_in_order() {
        let joined: Vec<&str> = THEMES
            .iter()
            .flat_map(|t| t.words.iter().copied())
            .collect();
        assert_eq!(joined, WORDS);
        assert_eq!(THEMES[0].id, "nature", "card #224's words come first");
        assert_eq!(
            NATURE.len(),
            1021,
            "card #224's words are kept as they were"
        );
        let mut ids = HashSet::new();
        for t in THEMES {
            assert!(ids.insert(t.id), "{} twice", t.id);
            assert!(!t.id.is_empty() && !t.label.is_empty());
            assert_eq!(theme(t.id), Some(t));
            assert!(!t.sample.is_empty());
            for w in t.sample {
                assert!(
                    t.words.contains(w),
                    "{}: sample {w} isn't one of its words",
                    t.id
                );
            }
        }
        assert_eq!(theme(""), None);
        assert_eq!(theme("nope"), None);
    }

    #[test]
    fn words_on_leaves_out_the_themes_turned_off() {
        let off = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(words_on(&[]), None, "all on: the full list");
        assert_eq!(words_on(&off(&["westerns"])), None, "an unknown theme");
        let all: Vec<&str> = THEMES.iter().map(|t| t.id).collect();
        assert_eq!(words_on(&off(&all)), None, "all off: the full list");
        let w = words_on(&off(&["nature", "movies"])).unwrap();
        let (nature, movies) = (theme("nature").unwrap(), theme("movies").unwrap());
        assert_eq!(
            w.len(),
            WORDS.len() - nature.words.len() - movies.words.len()
        );
        assert!(
            w.iter()
                .all(|x| !nature.words.contains(x) && !movies.words.contains(x))
        );
        assert!(w.contains(&"zeus") && w.contains(&"onion"));
    }

    #[test]
    fn pick_free_in_stays_in_its_theme() {
        let t = theme("myth").unwrap();
        let first = pick_free_in(t.words, "s1", &|_| false).unwrap();
        assert!(t.words.contains(&first.as_str()), "{first}");
        let second = pick_free_in(t.words, "s1", &|w| w == first).unwrap();
        assert!(t.words.contains(&second.as_str()) && second != first);
        assert_eq!(pick_free_in(t.words, "s1", &|w| t.words.contains(&w)), None);
        assert_eq!(pick_free_in(&[], "s1", &|_| false), None);
    }

    #[test]
    fn pick_free_probes_once() {
        let first = pick_free("s1", &|_| false).unwrap();
        assert_eq!(first, pick("s1", None));
        let second = pick_free("s1", &|w| w == first).unwrap();
        assert_ne!(second, first);
        assert_eq!(pick_free("s1", &|_| true), None);
    }
}
