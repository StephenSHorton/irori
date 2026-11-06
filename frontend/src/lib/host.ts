// The host: the Rust side's commands (src/gen/bindings.ts, generated from
// crates/window by `cargo xtask bindings`), its events, and the opener and
// clipboard plugins. The gateways in src/lib call the services here by
// their Go names with Go's positional arguments; components never import
// this file or the bindings.
//
// Conventions kept from the Wails build (surface.md 7.6): a failed call
// rejects with an Error whose message is the Rust side's text, and an
// event callback gets the payload as `data`.

import { Channel } from "@tauri-apps/api/core"
import { emit, listen } from "@tauri-apps/api/event"
import { readImage, writeText } from "@tauri-apps/plugin-clipboard-manager"
import { openUrl } from "@tauri-apps/plugin-opener"

import { commands } from "../gen/bindings"
import type * as B from "../gen/bindings"

// asError makes a Tauri rejection (the command's error text) an Error.
export function asError(err: unknown): Error {
  if (err instanceof Error) return err
  if (typeof err === "string") return new Error(err)
  if (err && typeof err === "object" && "message" in err) {
    return new Error(String((err as { message: unknown }).message))
  }
  try {
    return new Error(JSON.stringify(err))
  } catch {
    return new Error(String(err))
  }
}

// call runs one command, rejecting with an Error.
function call<T>(p: Promise<T>): Promise<T> {
  return p.catch((err: unknown) => {
    throw asError(err)
  })
}

// done is a command whose answer is nothing (Rust `()`, sent as null).
function done(p: Promise<null>): Promise<void> {
  return call(p).then(() => undefined)
}

// ---- events ---------------------------------------------------------------------

export type HostEvent<T> = { name: string; data: T }

// Events.On subscribes to a host event and returns a synchronous
// unsubscribe. Tauri's listen resolves later, so an unsubscribe that comes
// first is applied when it does.
export const Events = {
  // listening, when given, runs once the subscription is live.
  On<T = unknown>(
    name: string,
    callback: (ev: HostEvent<T>) => void,
    listening?: () => void
  ): () => void {
    let off: (() => void) | null = null
    let cancelled = false
    listen<T>(name, (e) => callback({ name, data: e.payload }))
      .then((unlisten) => {
        if (cancelled) unlisten()
        else {
          off = unlisten
          listening?.()
        }
      })
      .catch(() => {})
    return () => {
      cancelled = true
      off?.()
      off = null
    }
  },
  // Emit tells the host something happened here.
  Emit(name: string): void {
    void emit(name).catch(() => {})
  },
}

// WindowLog writes one line to irori.log (window-log.ts).
export function WindowLog(line: string): Promise<void> {
  return done(commands.windowLog(line))
}

// ---- Browser, Clipboard ---------------------------------------------------------

export const Browser = {
  OpenURL(url: string | URL): Promise<void> {
    return call(openUrl(String(url)))
  },
}

export const Clipboard = {
  SetText(text: string): Promise<void> {
    return call(writeText(text))
  },
  // Image is the picture on the clipboard as RGBA pixels, read natively
  // (card #321): WebKit hands a paste of a HEIC-only clipboard (a macOS
  // screenshot) to the page with no files.
  async Image(): Promise<{ rgba: Uint8Array; width: number; height: number }> {
    const img = await call(readImage())
    try {
      const [rgba, size] = await Promise.all([img.rgba(), img.size()])
      return { rgba, width: size.width, height: size.height }
    } finally {
      void img.close().catch(() => {})
    }
  },
}

// ---- services ---------------------------------------------------------------------

export const RosterService = {
  Agents: () => call(commands.rosterAgents()),
  Device: () => call(commands.rosterDevice()),
  Version: () => call(commands.rosterVersion()),
  Recent: (limit: number) => call(commands.rosterRecent(limit)),
  Gone: (limit: number) => call(commands.rosterGone(limit)),
  Revive: (sessionId: string) => call(commands.rosterRevive(sessionId)),
  // Check slots (card #437): this device's check.sh runs holding a slot.
  CheckSlots: () => call(commands.rosterCheckSlots()),
  // Stop (card #207): cuts the agent's running turn; text is its next prompt.
  Stop: (key: string, text: string) => done(commands.agentStop(key, text)),
}

export const InboxService = {
  List: (limit: number) => call(commands.inboxList(limit)),
  Held: () => call(commands.inboxHeld()),
  HeldHistory: (limit: number) => call(commands.inboxHeldHistory(limit)),
  Counts: () => call(commands.inboxCounts()),
  Get: (deliveryId: string) => call(commands.inboxGet(deliveryId)),
  Send: (to: string, repo: string, text: string, mentions: B.Mention[]) =>
    call(commands.inboxSend(to, repo, text, mentions)),
  Reply: (deliveryId: string, text: string, mentions: B.Mention[]) =>
    call(commands.inboxReply(deliveryId, text, mentions)),
  // Stop and read (card #207): the sender's running turn is cut.
  ReplyStop: (deliveryId: string, text: string) =>
    call(commands.inboxReplyStop(deliveryId, text)),
  ReplyWithFiles: (
    deliveryId: string,
    text: string,
    mentions: B.Mention[],
    files: B.UploadFile[]
  ) => call(commands.inboxReplyWithFiles(deliveryId, text, mentions, files)),
  MarkRead: (deliveryId: string) => done(commands.inboxMarkRead(deliveryId)),
  Release: (deliveryId: string) => done(commands.inboxRelease(deliveryId)),
  Dismiss: (deliveryId: string) => done(commands.inboxDismiss(deliveryId)),
  // Card #488: "Start an agent in <repo>" on your message nobody there can
  // take yet; the answer is the card it became ("#12").
  StartAgent: (deliveryId: string) =>
    call(commands.inboxStartAgent(deliveryId)),
  RepoNames: () => call(commands.inboxRepoNames()),
  AllowAndRelease: (deliveryId: string) =>
    done(commands.inboxAllowAndRelease(deliveryId)),
  Policy: () => call(commands.inboxPolicy()),
  SetPolicy: (p: B.PolicyInput) => done(commands.inboxSetPolicy(p)),
  SetPaused: (paused: boolean) => done(commands.inboxSetPaused(paused)),
}

// Wind down all (card #220): every agent saves its place, hands back its
// cards and stops; Pause all turns on after, if asked.
export const WindDownService = {
  All: (scope: string, pauseAfter: boolean) =>
    call(commands.windDownAll(scope, pauseAfter)),
  Status: () => call(commands.windDownStatus()),
  Stop: (key: string) => done(commands.windDownStop(key)),
}

// The notification history (card #377).
export const NotificationService = {
  List: () => call(commands.notificationsList()),
  Unread: () => call(commands.notificationsUnread()),
  Read: (keys: string[]) => done(commands.notificationsRead(keys)),
  AnswerAsk: (askId: string, choice: string) =>
    done(commands.notificationsAnswerAsk(askId, choice)),
}

export const BoardService = {
  Cards: () => call(commands.boardCards()),
  IdleReport: () => call(commands.boardIdleReport()),
  Card: (id: string) => call(commands.boardCard(id)),
  CardsById: (ids: string[]) => call(commands.boardCardsById(ids)),
  DoneCards: (limit: number, before: string) =>
    call(commands.boardDoneCards(limit, before)),
  DoneCardsIn: (limit: number, before: string, scope: string) =>
    call(commands.boardDoneCardsIn(limit, before, scope)),
  ClearDoneCount: (olderThanSecs: number, scope: string) =>
    call(commands.boardClearDoneCount(olderThanSecs, scope)),
  ClearDone: (olderThanSecs: number, ids: string[], scope: string) =>
    call(commands.boardClearDone(olderThanSecs, ids, scope)),
  Message: (taskId: string, msgId: string) =>
    call(commands.boardMessage(taskId, msgId)),
  Comment: (id: string, text: string, tell: string, mentions: B.Mention[]) =>
    call(commands.boardComment(id, text, tell, mentions)),
  CommentWithFiles: (
    id: string,
    text: string,
    tell: string,
    mentions: B.Mention[],
    files: B.UploadFile[]
  ) => call(commands.boardCommentWithFiles(id, text, tell, mentions, files)),
  Answer: (
    taskId: string,
    blockerId: string,
    option: number,
    text: string,
    wake: boolean,
    mentions: B.Mention[]
  ) =>
    call(commands.boardAnswer(taskId, blockerId, option, text, wake, mentions)),
  AnswerWithFiles: (
    taskId: string,
    blockerId: string,
    option: number,
    text: string,
    wake: boolean,
    mentions: B.Mention[],
    files: B.UploadFile[]
  ) =>
    call(
      commands.boardAnswerWithFiles(
        taskId,
        blockerId,
        option,
        text,
        wake,
        mentions,
        files
      )
    ),
  ResolveBlocker: (
    taskId: string,
    blockerId: string,
    note: string,
    mentions: B.Mention[]
  ) => call(commands.boardResolveBlocker(taskId, blockerId, note, mentions)),
  MentionCandidates: () => call(commands.boardMentionCandidates()),
  AskAgain: (id: string) => done(commands.boardAskAgain(id)),
  // "Start an agent with this" (card #452).
  StartAgent: (id: string, note: string) =>
    done(commands.boardStartAgent(id, note)),
  HandOn: (id: string) => call(commands.boardHandOn(id)),
  AskRepos: () => call(commands.boardAskRepos()),
  CloseAsk: (id: string, note: string) =>
    call(commands.boardCloseAsk(id, note)),
  Close: (id: string, state: string, note: string) =>
    call(commands.boardClose(id, state, note)),
  Delete: (id: string) => done(commands.boardDelete(id)),
  Pin: (id: string, pinned: boolean) => done(commands.boardPin(id, pinned)),
  // Pause / Resume (card #207): who was told.
  Pause: (id: string, paused: boolean, note: string) =>
    call(commands.boardPause(id, paused, note)),
  MovePin: (id: string, before: string) =>
    done(commands.boardMovePin(id, before)),
  Place: (id: string, x: number, y: number) =>
    done(commands.boardPlace(id, x, y)),
  Create: (input: B.NewCardInput) => call(commands.boardCreate(input)),
  StartThread: (
    text: string,
    mentions: B.Mention[],
    boardId: string,
    agentsNow: boolean | null
  ) => call(commands.boardStartThread(text, mentions, boardId, agentsNow)),
  StartThreadWithFiles: (
    text: string,
    mentions: B.Mention[],
    files: B.UploadFile[],
    agentsNow: boolean | null
  ) =>
    call(commands.boardStartThreadWithFiles(text, mentions, files, agentsNow)),
  ThreadAgentsNow: (id: string, on: boolean) =>
    done(commands.boardThreadAgentsNow(id, on)),
  Threads: (boardId: string) => call(commands.boardThreads(boardId)),
  ThreadsAll: () => call(commands.boardThreadsAll()),
  CardRef: (r: string) => call(commands.boardCardRef(r)),
  ResolveThread: (id: string) => done(commands.boardResolveThread(id)),
  ReopenThread: (id: string) => done(commands.boardReopenThread(id)),
  CommentModify: (id: string, commentId: string) =>
    done(commands.boardCommentModify(id, commentId)),
  CommentModifyCancel: (id: string, commentId: string) =>
    done(commands.boardCommentModifyCancel(id, commentId)),
  CommentModifySave: (id: string, commentId: string, text: string) =>
    done(commands.boardCommentModifySave(id, commentId, text)),
  Reopen: (id: string) => call(commands.boardReopen(id)),
}

// TalkService is Talk's channels, DMs and posts (cards #389, #390).
export const TalkService = {
  Conversations: () => call(commands.talkConversations()),
  Posts: (convId: string, before: string | null, limit: number) =>
    call(commands.talkPosts(convId, before, limit)),
  // mentions: the keys the composer's picker recorded (#461).
  Post: (
    convId: string,
    text: string,
    mentions: string[],
    agentsNow: boolean,
    threadFrom: string | null
  ) => call(commands.talkPost(convId, text, mentions, agentsNow, threadFrom)),
  Read: (convId: string, upTo: string) => done(commands.talkRead(convId, upTo)),
  LinkCard: (postId: string, taskId: string) =>
    done(commands.talkLinkCard(postId, taskId)),
  // A thread about a channel's message (#454): it records the channel and
  // its repo, and the channel gets the one-line pointer.
  ThreadFrom: (convId: string, postId: string, text: string) =>
    call(commands.talkThreadFrom(convId, postId, text)),
  // spaceId puts the channel in a space, where its members see it (#392).
  ChannelCreate: (
    name: string,
    topic: string | null,
    spaceId: string | null = null
  ) => call(commands.talkChannelCreate(name, topic, spaceId)),
  Dm: (member: B.TalkMemberRef) => call(commands.talkDm(member)),
  Join: (convId: string, member: B.TalkMemberRef) =>
    done(commands.talkJoin(convId, member)),
  Leave: (convId: string, member: B.TalkMemberRef) =>
    done(commands.talkLeave(convId, member)),
}

// NotesService is the window's notes_* commands (card #411).
export const NotesService = {
  List: (scope: string, removed = false) =>
    call(commands.notesList(scope, removed)),
  Waiting: () => call(commands.notesWaiting()),
  New: (note: B.NoteInput) => call(commands.noteNew(note)),
  Accept: (id: string) => call(commands.noteAccept(id)),
  Edit: (id: string, edit: B.NoteEditInput) =>
    call(commands.noteEdit(id, edit)),
  Remove: (id: string, why: string) => call(commands.noteRemove(id, why)),
}

// AgentsService is the window's agents_* commands (card #449): Trust and
// start for a card whose repo isn't trusted, and a board's permissions;
// and (card #448) a board's spawning switch, model and effort.
export const AgentsService = {
  Settings: (boardId: string) => call(commands.agentsSettings(boardId)),
  Set: (change: B.AgentsChange) => call(commands.agentsSet(change)),
  End: (key: string) => done(commands.agentsEnd(key)),
  OfferSet: (key: string, on: boolean) =>
    done(commands.agentsOfferSet(key, on)),
  Logs: (key: string) => call(commands.agentsLogs(key)),
  Attach: (key: string, cols: number, rows: number) =>
    call(commands.agentsAttach(key, cols, rows)),
  Running: () => call(commands.agentsRunning()),
  TrustNeed: (taskId: string) => call(commands.agentsTrustNeed(taskId)),
  TrustAndStart: (taskId: string) => call(commands.agentsTrustAndStart(taskId)),
  // Asks the device the agent starts on to trust it there (part 11);
  // writes no trust. That device's name.
  TrustRequest: (taskId: string) => call(commands.agentsTrustRequest(taskId)),
  Trusted: () => call(commands.agentsTrusted()),
  TrustRepo: (repoKey: string) => call(commands.agentsTrustRepo(repoKey)),
  Permissions: (boardId: string) => call(commands.agentsPermissions(boardId)),
  SetPermissions: (p: B.AgentPermissions) =>
    call(commands.agentsPermissionsSet(p)),
}

export const BoardsService = {
  List: () => call(commands.boardsList()),
  Create: (name: string, tag: string) => call(commands.boardsCreate(name, tag)),
  Rename: (boardId: string, name: string) =>
    call(commands.boardsRename(boardId, name)),
  AddToSpace: (spaceId: string, name: string, tag: string) =>
    call(commands.boardsAddToSpace(spaceId, name, tag)),
  RenameInSpace: (spaceId: string, boardId: string, name: string) =>
    call(commands.boardsRenameInSpace(spaceId, boardId, name)),
  Personal: () => call(commands.boardsPersonal()),
  AddToPersonal: (name: string, tag: string) =>
    call(commands.boardsAddToPersonal(name, tag)),
  RenameInPersonal: (boardId: string, name: string) =>
    call(commands.boardsRenameInPersonal(boardId, name)),
  Pin: (boardId: string, pinned: boolean) =>
    done(commands.boardsPin(boardId, pinned)),
  Leave: (boardId: string) => done(commands.boardsLeave(boardId)),
  Forget: (boardId: string) => done(commands.boardsForget(boardId)),
  ForgetNow: (boardId: string) => done(commands.boardsForgetNow(boardId)),
  RemoveMember: (boardId: string, personId: string) =>
    call(commands.boardsRemoveMember(boardId, personId)),
  SetRole: (boardId: string, personId: string, role: string) =>
    call(commands.boardsSetRole(boardId, personId, role)),
  SharePreview: (taskId: string, boardId: string) =>
    call(commands.boardsSharePreview(taskId, boardId)),
  Share: (taskId: string, boardId: string, hash: string) =>
    call(commands.boardsShare(taskId, boardId, hash)),
  Withdraw: (taskId: string) => call(commands.boardsWithdraw(taskId)),
  TransferOwner: (taskId: string, to: string) =>
    call(commands.boardsTransferOwner(taskId, to)),
  Settings: (boardId: string) => call(commands.boardsSettings(boardId)),
  SetReceive: (boardId: string, preset: string, people: string[]) =>
    call(commands.boardsSetReceive(boardId, preset, people)),
  SetAutonomy: (boardId: string, preset: string) =>
    call(commands.boardsSetAutonomy(boardId, preset)),
  SetThreads: (boardId: string, mode: string) =>
    call(commands.boardsSetThreads(boardId, mode)),
  SetReviewer: (boardId: string, agent: string, repo: string) =>
    call(commands.boardsSetReviewer(boardId, agent, repo)),
  Invite: (boardId: string) => call(commands.boardsInvite(boardId)),
  JoinInvite: (link: string) => call(commands.boardsJoinInvite(link)),
  ConfirmInviteWords: (words: string) =>
    call(commands.boardsConfirmInviteWords(words)),
  InviteStatus: () => call(commands.boardsInviteStatus()),
  CancelInvite: () => done(commands.boardsCancelInvite()),
}

export const DevicesService = {
  Devices: () => call(commands.devicesDevices()),
  Colors: () => call(commands.devicesColors()),
  SetDeviceColor: (deviceId: string, color: string) =>
    done(commands.devicesSetDeviceColor(deviceId, color)),
  PersonColor: () => call(commands.devicesPersonColor()),
  SetPersonColor: (color: string) =>
    done(commands.devicesSetPersonColor(color)),
  AgentAvatars: () => call(commands.devicesAgentAvatars()),
  PersonAvatar: () => call(commands.devicesPersonAvatar()),
  SetPersonAvatar: (avatar: string) =>
    done(commands.devicesSetPersonAvatar(avatar)),
  RenameDevice: (deviceId: string, name: string) =>
    call(commands.devicesRenameDevice(deviceId, name)),
  SideNames: () => call(commands.devicesSideNames()),
  SetDeviceSides: (sides: string[]) => call(commands.devicesSetDeviceSides(sides)),
  AddDevice: (role: string) => call(commands.devicesAddDevice(role)),
  JoinWithCode: (code: string, server: string) =>
    call(commands.devicesJoinWithCode(code, server)),
  ConfirmWords: (words: string) => done(commands.devicesConfirmWords(words)),
  ConfirmKey: (key: string) => done(commands.devicesConfirmKey(key)),
  PairingStatus: () => call(commands.devicesPairingStatus()),
  CancelPairing: () => done(commands.devicesCancelPairing()),
  RemoveDevice: (deviceId: string) =>
    done(commands.devicesRemoveDevice(deviceId)),
  ResetDevice: () => done(commands.devicesResetDevice()),
  HomeServer: () => call(commands.devicesHomeServer()),
  SetHomeServer: (on: boolean, outside: boolean, publicAddr: string) =>
    call(commands.devicesSetHomeServer(on, outside, publicAddr)),
  SetHomeServerAwake: (on: boolean) =>
    call(commands.devicesSetHomeServerAwake(on)),
  SetPublicRelays: (on: boolean) => call(commands.devicesSetPublicRelays(on)),
  Relay: () => call(commands.devicesRelay()),
  SetRelay: (credentialUrl: string) =>
    call(commands.devicesSetRelay(credentialUrl)),
}

export const SettingsService = {
  GetNotifications: () => call(commands.settingsGetNotifications()),
  SetNotifications: (o: B.NotifyOptions) =>
    call(commands.settingsSetNotifications(o)),
  TestNotification: () => done(commands.settingsTestNotification()),
  GetLaunchAtLogin: () => call(commands.settingsGetLaunchAtLogin()),
  SetLaunchAtLogin: (enabled: boolean) =>
    done(commands.settingsSetLaunchAtLogin(enabled)),
  GetYourName: () => call(commands.settingsGetYourName()),
  SetYourName: (name: string) => call(commands.settingsSetYourName(name)),
}

export const AttachmentsService = {
  Info: (sha: string) => call(commands.attachmentsInfo(sha)),
  Fetch: (sha: string) => call(commands.attachmentsFetch(sha)),
  Save: (sha: string) => call(commands.attachmentsSave(sha)),
  Open: (sha: string) => done(commands.attachmentsOpen(sha)),
}

export const UpdateService = {
  GetStatus: () => call(commands.updateGetStatus()),
  GetVersion: () => call(commands.updateGetVersion()),
  Supported: () => call(commands.updateSupported()),
  CheckAndDownload: () => call(commands.updateCheckAndDownload()),
  RestartToUpdate: () => done(commands.updateRestartToUpdate()),
}

// Voice input (card #240): Whisper on this device; words arrive as
// voice:event, the one-time model download as voice:download.
export const VoiceService = {
  Status: () => call(commands.voiceStatus()),
  DownloadModel: () => done(commands.voiceDownloadModel()),
  Start: () => call(commands.voiceStart()),
  Stop: (session: number) => done(commands.voiceStop(session)),
  OpenSettings: (code: string) => done(commands.voiceOpenSettings(code)),
}

// Card terminals (card #369; inline-terminals.md section 5). Frames come as
// raw ArrayBuffers on their own Channel, never as events; everything else
// is a TermMsg on a second one.
export const TermService = {
  Folders: (card: string) => call(commands.termFolders(card)),
  List: (card: string) => call(commands.termList(card)),
  Open: (scope: B.Scope, cols: number, rows: number, cwd: string | null) =>
    call(commands.termOpen(scope, cols, rows, cwd)),
  Attach(
    id: number,
    onFrame: (frame: ArrayBuffer) => void,
    onMsg: (msg: B.TermMsg) => void
  ): Promise<void> {
    const frames = new Channel<ArrayBuffer>()
    frames.onmessage = onFrame
    const msgs = new Channel<B.TermMsg>()
    msgs.onmessage = onMsg
    return done(commands.termAttach(id, frames, msgs))
  },
  Detach: (id: number) => done(commands.termDetach(id)),
  Write: (id: number, data: string) => done(commands.termWrite(id, data)),
  Paste: (id: number, text: string) => done(commands.termPaste(id, text)),
  Resize: (id: number, cols: number, rows: number) =>
    done(commands.termResize(id, cols, rows)),
  Scroll: (id: number, delta: number) => done(commands.termScroll(id, delta)),
  CellPx: (id: number, w: number, h: number) =>
    done(commands.termCellPx(id, w, h)),
  Focus: (id: number, on: boolean) => done(commands.termFocus(id, on)),
  Redraw: (id: number) => done(commands.termRedraw(id)),
  Ack: (id: number, seq: number) => done(commands.termAck(id, seq)),
  Close: (id: number) => done(commands.termClose(id)),
}

// Merge desks (card #417).
export const DeskService = {
  List: () => call(commands.deskList()),
  Set: (s: B.DeskSettings) => done(commands.deskSet(s)),
  Hold: (repoKey: string, pr: number, note: string, cleared: boolean) =>
    done(commands.deskHold(repoKey, pr, note, cleared)),
  Pause: (repoKey: string, paused: boolean) =>
    done(commands.deskPause(repoKey, paused)),
  Ok: (repoKey: string, pr: number, headSha: string) =>
    done(commands.deskOk(repoKey, pr, headSha)),
}
