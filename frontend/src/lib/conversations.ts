// Talk's channels and DMs (cards #389, #390; docs/design/talk-channels.md):
// the gateway the Talk page reads. Threads keep their own (lib/cards'
// useThreads), since they carry the ask's state; conversations() lists
// only channels and DMs. In a plain browser everything runs on the sample
// below, and posting changes it in memory so the page can be tried.

import { useCallback, useEffect } from "react"

import { Events, TalkService } from "./host"
import {
  TalkConversation,
  TalkPost,
  type TalkMember,
  type TalkMemberRef,
} from "./models"
import { inWails } from "./irori"
import { useLatest } from "./talk"

export type Conversation = TalkConversation
export type Post = TalkPost
export type Member = TalkMember
export type MemberRef = TalkMemberRef

// EventTalkChanged fires on every talk.* row the Core folds.
export const EventTalkChanged = "talk:changed"

// PostsPage is how many posts a conversation loads at a time.
export const PostsPage = 100

// agentsNowFor is the default for a post's "Agents answer now" (design §3):
// on when the text names nobody or names an agent, off when it names only
// people. Card #391 adds the toggle that flips it.
export function agentsNowFor(mentioned: readonly { kind: string }[]): boolean {
  if (mentioned.length === 0) return true
  return mentioned.some((m) => m.kind === "agent")
}

// listed sorts channels by name and DMs newest first, as the Talk list
// shows them; threads are left out (they come from useThreads).
export function listed(convs: readonly Conversation[]): {
  channels: Conversation[]
  dms: Conversation[]
} {
  const channels = convs
    .filter((c) => c.kind === "channel")
    .sort((a, b) => a.name.localeCompare(b.name))
  const dms = convs
    .filter((c) => c.kind === "dm")
    .sort((a, b) => (a.lastAt < b.lastAt ? 1 : a.lastAt > b.lastAt ? -1 : 0))
  return { channels, dms }
}

// dmWith is the DM's other side (an agent DM has one), or the two agents of
// a read-only agent↔agent DM.
export function dmWith(c: Conversation): Member[] {
  return c.members.filter((m) => m.kind !== "person")
}

// dmGone is whether a DM's agents have all gone: none is on the live
// roster (by key, its status), or the roster calls it gone. Such a DM goes
// in Talk's Archive (card #473).
export function dmGone(c: Conversation, live: ReadonlyMap<string, { status: string }>): boolean {
  const other = dmWith(c)
  return other.length > 0 && other.every((m) => (live.get(m.key)?.status ?? "gone") === "gone")
}

// talkWaiting counts the conversations that wait on the person (card
// #484): their DMs with something new, except with agents that have gone
// (those wait in the Archive, not on them), and the channels that mention
// them. Talk's count adds it to the threads waiting on the person, on the
// sidebar and the phone's tab alike.
export function talkWaiting(
  convs: readonly Conversation[],
  live: ReadonlyMap<string, { status: string }>
): number {
  return convs.filter((c) => {
    if (c.kind === "channel") return c.mentions > 0
    return c.kind === "dm" && !c.readOnly && c.unread > 0 && !dmGone(c, live)
  }).length
}

// readAll marks each conversation with something unread read up to its
// newest post: the Archive's Mark all read (card #473). One that fails is
// left unread; the rest still go.
export async function readAll(convs: readonly Conversation[]): Promise<void> {
  await Promise.allSettled(
    convs
      .filter((c) => !c.readOnly && c.unread > 0)
      .map(async (c) => {
        const posts = await talkApi.posts(c.convId)
        const last = posts[posts.length - 1]?.postId
        if (last) await talkApi.read(c.convId, last)
      })
  )
}

export const talkApi = {
  conversations: (): Promise<Conversation[]> =>
    inWails() ? TalkService.Conversations() : Promise.resolve(sampleConvs()),
  posts: (convId: string, before: string | null = null): Promise<Post[]> =>
    inWails()
      ? TalkService.Posts(convId, before, PostsPage)
      : Promise.resolve(samplePosts(convId)),
  // mentions are the picker's keys: the agent picked, another person's in
  // a space too, whatever else answers to its name (#461).
  post: (
    convId: string,
    text: string,
    agentsNow: boolean,
    threadFrom: string | null = null,
    mentions: string[] = []
  ) =>
    inWails()
      ? TalkService.Post(convId, text, mentions, agentsNow, threadFrom)
      : Promise.resolve(samplePost(convId, text, agentsNow, threadFrom)),
  // linkCard marks the card made from a post (Make a card): its chip.
  linkCard: (postId: string, taskId: string): Promise<void> =>
    inWails() ? TalkService.LinkCard(postId, taskId) : Promise.resolve(sampleLink(postId, taskId)),
  read: (convId: string, upTo: string): Promise<void> =>
    inWails() ? TalkService.Read(convId, upTo) : Promise.resolve(sampleRead(convId)),
  // createChannel makes a channel in a shared space (its id), or in
  // Personal with none (card #461).
  createChannel: (name: string, topic: string | null, spaceId: string | null = null): Promise<string> =>
    inWails()
      ? TalkService.ChannelCreate(name, topic, spaceId)
      : Promise.resolve(sampleChannel(name, topic, spaceId)),
  dm: (member: MemberRef): Promise<string> =>
    inWails() ? TalkService.Dm(member) : Promise.resolve(sampleDm(member)),
  join: (convId: string, member: MemberRef): Promise<void> =>
    inWails() ? TalkService.Join(convId, member) : Promise.resolve(sampleJoin(convId, member)),
  leave: (convId: string, member: MemberRef): Promise<void> =>
    inWails() ? TalkService.Leave(convId, member) : Promise.resolve(sampleLeave(convId, member)),
}

// useTalkFresh reloads on every Talk change, on focus, and on the sample's.
function useTalkFresh(reload: () => Promise<void>) {
  useEffect(() => {
    void reload()
    if (!inWails()) {
      sampleListeners.add(reload)
      return () => {
        sampleListeners.delete(reload)
      }
    }
    const off = Events.On(EventTalkChanged, () => void reload())
    const onFocus = () => void reload()
    window.addEventListener("focus", onFocus)
    return () => {
      off()
      window.removeEventListener("focus", onFocus)
    }
  }, [reload])
}

export function useConversations(): {
  conversations: Conversation[]
  error: string | null
} {
  const load = useCallback(() => talkApi.conversations(), [])
  const { value, error, reload } = useLatest<Conversation[]>(load, [])
  useTalkFresh(reload)
  return { conversations: value, error }
}

export function usePosts(convId: string): {
  posts: Post[]
  loading: boolean
  error: string | null
} {
  const load = useCallback(
    () => (convId ? talkApi.posts(convId) : Promise.resolve([])),
    [convId]
  )
  const { value, loading, error, reload } = useLatest<Post[]>(load, [])
  useTalkFresh(reload)
  return { posts: value, loading, error }
}

// ---------- the browser preview's sample ----------

const me = "person:sample-person"
const heron = "sample-device/sample02-synthetic-session"
const otter = "sample-device/sample01-synthetic-session"
const badger = "sample-device/sample03-synthetic-session"

const sampleListeners = new Set<() => Promise<void>>()
function sampleChanged() {
  for (const fn of sampleListeners) void fn()
}

const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString()

type SampleConv = Conversation & { posts: Post[]; readUpTo: string }

let sample: SampleConv[] | null = null

function post(convId: string, n: number, by: string, min: number, text: string, extra: Partial<Post> = {}): Post {
  return TalkPost({ postId: `${convId}-p${n}`, convId, by, text, at: ago(min), ...extra })
}

function sampleState(): SampleConv[] {
  if (sample) return sample
  const ch = (id: string, name: string, topic: string, members: Member[], posts: Post[], read = 0): SampleConv => ({
    ...TalkConversation({ convId: id, kind: "channel", name, topic, members }),
    posts,
    readUpTo: posts[read - 1]?.postId ?? "",
  })
  const you: Member = { kind: "person", key: me, name: "you" }
  const a = (key: string, name: string): Member => ({ kind: "agent", key, name })
  sample = [
    ch(
      "sample-ch-suzuri",
      "suzuri",
      "The terminal move",
      [you, a(heron, "sample-heron"), a(otter, "sample-otter")],
      [
        post("sample-ch-suzuri", 1, otter, 300, "Windows installer now takes over Go's install: #3 is on it."),
        post("sample-ch-suzuri", 2, me, 40, "Is anything still blocking the move? @sample-heron", { mentions: [heron] }),
        post("sample-ch-suzuri", 3, heron, 38, "Only the live PC transfer. It needs one Firewall click from you, or we wait for #8's install."),
        post("sample-ch-suzuri", 5, me, 36, "#20", { threadFrom: "sample-ch-suzuri-p3" }),
        post("sample-ch-suzuri", 4, me, 30, "Let's tag 0.10.0 the moment #8 merges, not before."),
      ],
      2
    ),
    ch(
      "sample-ch-releases",
      "releases",
      "",
      [you, a(otter, "sample-otter")],
      [
        post("sample-ch-releases", 1, me, 90, "0.10.2 looked good on the phone."),
        post("sample-ch-releases", 2, otter, 12, "Released 0.10.2; the PC and the phone report it. @you", { mentions: [me] }),
      ],
      1
    ),
    ch("sample-ch-irori", "irori", "Irori itself", [you, a(badger, "sample-badger")], []),
    // A channel in the MLG space (card #461): ?space=sample-board-mlg shows it alone.
    {
      ...ch(
        "sample-ch-netcode",
        "netcode",
        "Rollback and the lobby",
        [you, a(otter, "sample-otter")],
        [post("sample-ch-netcode", 1, otter, 60, "Rollback holds at 8 frames on the LAN test.")],
        1
      ),
      spaceId: "sample-board-mlg",
    },
    {
      ...TalkConversation({ convId: "sample-dm-heron", kind: "dm", name: "sample-heron", members: [you, a(heron, "sample-heron")] }),
      posts: [
        post("sample-dm-heron", 1, me, 200, "How would you feel about dropping the workspace panel altogether?"),
        post("sample-dm-heron", 2, heron, 199, "Happy to. It's the only thing that needs Go's binary. I'd make it a card so others see it."),
        post("sample-dm-heron", 3, heron, 5, "The panel's gone and the PC check passed. Anything else before I go idle?"),
      ],
      readUpTo: "sample-dm-heron-p2",
    },
    {
      ...TalkConversation({
        convId: "sample-dm-otter-badger",
        kind: "dm",
        name: "sample-otter ↔ sample-badger",
        members: [a(otter, "sample-otter"), a(badger, "sample-badger")],
        readOnly: true,
      }),
      posts: [
        post("sample-dm-otter-badger", 1, otter, 25, "Can you take the docs half of #4? I'm deep in the parser."),
        post("sample-dm-otter-badger", 2, badger, 24, "On it."),
      ],
      readUpTo: "sample-dm-otter-badger-p2",
    },
  ]
  // Many more, as a busy week leaves them (card #463): DMs with agents
  // that have gone, and agent↔agent DMs the person only reads.
  // Agents that finished their cards, named as Irori names spawned agents
  // (card-<n>-1), with the last thing each said.
  const lastWords = [
    "On main as 4c1e9a2. The phone build passed too; ending my session.",
    "Done check raised with screenshots. Nothing else from me.",
    "Rebased onto main and pushed; the merge desk has it.",
    "The flaky capture test was a timeout on one thread. Fixed, on main.",
    "Settings › Agents now shows the model picker. Proof is on the card.",
    "Closed as a duplicate of #452; I linked the two.",
    "Release notes for 0.11.0 are in docs/releasing.md.",
    "Card split into #441 and #442; both are on the Board unheld.",
    "The Windows installer check passed on the PC. Going idle.",
    "Moved the search index to its own thread; Talk's list no longer stutters.",
    "Left a note on the card for whoever picks up the Android half.",
    "Merged. The old workspace panel is gone from the sidebar.",
    "Answered on the blocker; waiting on nothing now.",
    "On main. The notification sound is off by default, as you asked.",
  ]
  lastWords.forEach((text, i) => {
    const n = 470 - i * 3
    const id = `sample-dm-gone-${i + 1}`
    const key = `sample-gone-${i + 1}`
    sample!.push({
      ...TalkConversation({ convId: id, kind: "dm", name: key, members: [you, a(key, `card-${n}-1`)] }),
      posts: [
        post(id, 1, me, 460 + i * 95, `How's #${n} going?`),
        post(id, 2, key, 400 + i * 95, text),
      ],
      readUpTo: `${id}-p2`,
    })
  })
  // Two gone agents left messages the person hasn't read: they wait in the
  // Archive, marked unread, and count beside "Archive" (card #473).
  sample.push({
    ...TalkConversation({ convId: "sample-dm-gone-new", kind: "dm", name: "sample-gone-new", members: [you, a("sample-gone-new", "card-474-1")] }),
    posts: [
      post("sample-dm-gone-new", 1, me, 50, "Is #474 on main?"),
      post("sample-dm-gone-new", 2, "sample-gone-new", 40, "Merged as #474. My session ends here."),
    ],
    readUpTo: "sample-dm-gone-new-p1",
  })
  sample.push({
    ...TalkConversation({ convId: "sample-dm-gone-new2", kind: "dm", name: "sample-gone-new2", members: [you, a("sample-gone-new2", "card-475-1")] }),
    posts: [
      post("sample-dm-gone-new2", 1, "sample-gone-new2", 25, "The PC check found a second copy of the bug in the phone tabs; I fixed both."),
    ],
    readUpTo: "",
  })
  for (let i = 1; i <= 24; i++) {
    const id = `sample-dm-aa-${i}`
    const x = `sample-aa-${i}`
    sample.push({
      ...TalkConversation({
        convId: id,
        kind: "dm",
        name: `${x} ↔ sample-otter`,
        members: [a(x, `agent-${i}`), a(otter, "sample-otter")],
        readOnly: true,
      }),
      posts: [post(id, 1, x, 60 + i * 30, `Rebased #${400 + i} onto yours.`)],
      readUpTo: "",
    })
  }
  return sample
}

function counted(c: SampleConv): Conversation {
  const at = c.posts.findIndex((p) => p.postId === c.readUpTo)
  const fresh = c.posts.slice(at + 1).filter((p) => p.by !== me)
  const { posts, readUpTo, ...conv } = c
  void readUpTo
  return {
    ...conv,
    unread: fresh.length,
    mentions: fresh.filter((p) => p.mentions.includes(me)).length,
    lastAt: posts[posts.length - 1]?.at ?? "",
  }
}

function sampleConvs(): Conversation[] {
  return sampleState().map(counted)
}

function samplePosts(convId: string): Post[] {
  return sampleState().find((c) => c.convId === convId)?.posts ?? []
}

function samplePost(convId: string, text: string, agentsNow: boolean, threadFrom: string | null): string {
  const c = sampleState().find((x) => x.convId === convId)
  if (!c) throw new Error(`talk: no conversation ${convId}`)
  const p = post(convId, c.posts.length + 1, me, 0, text, { agentsNow, threadFrom })
  c.posts.push(p)
  c.readUpTo = p.postId
  sampleChanged()
  return p.postId
}

function sampleLink(postId: string, taskId: string) {
  for (const c of sampleState()) {
    const p = c.posts.find((x) => x.postId === postId)
    if (p) {
      p.card = taskId
      sampleChanged()
      return
    }
  }
}

// fromPost is the New card form a message fills (Make a card): its first
// line as the title, the message and where it was said as the objective.
export function fromPost(text: string, where: string): { title: string; objective: string } {
  const first = text.trim().split(/\r?\n/)[0] ?? ""
  const title = first.length > 80 ? `${first.slice(0, 79).trimEnd()}…` : first
  return { title, objective: `${text.trim()}\n\n(From a message in ${where}.)` }
}

function sampleRead(convId: string) {
  const c = sampleState().find((x) => x.convId === convId)
  if (!c || !c.posts.length) return
  const last = c.posts[c.posts.length - 1].postId
  if (c.readUpTo === last) return
  c.readUpTo = last
  sampleChanged()
}

function sampleChannel(name: string, topic: string | null, spaceId: string | null = null): string {
  // Names are per space (card #461), so the id carries the space.
  const id = spaceId ? `sample-ch-${spaceId}-${name}` : `sample-ch-${name}`
  if (!sampleState().some((c) => c.convId === id)) {
    sampleState().push({
      ...TalkConversation({
        convId: id,
        kind: "channel",
        name,
        topic,
        members: [{ kind: "person", key: me, name: "you" }],
        ...(spaceId ? { spaceId } : {}),
      }),
      posts: [],
      readUpTo: "",
    })
    sampleChanged()
  }
  return id
}

function sampleDm(member: MemberRef): string {
  const id = `sample-dm-${member.key}`
  const found = sampleState().find((c) => c.kind === "dm" && c.members.some((m) => m.key === member.key) && !c.readOnly)
  if (found) return found.convId
  sampleState().push({
    ...TalkConversation({
      convId: id,
      kind: "dm",
      name: member.key,
      members: [{ kind: "person", key: me, name: "you" }, { ...member }],
    }),
    posts: [],
    readUpTo: "",
  })
  sampleChanged()
  return id
}

function sampleJoin(convId: string, member: MemberRef) {
  const c = sampleState().find((x) => x.convId === convId)
  if (!c || c.members.some((m) => m.key === member.key)) return
  c.members = [...c.members, { ...member }]
  sampleChanged()
}

function sampleLeave(convId: string, member: MemberRef) {
  const c = sampleState().find((x) => x.convId === convId)
  if (!c) return
  c.members = c.members.filter((m) => m.key !== member.key)
  sampleChanged()
}
