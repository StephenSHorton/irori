// The host: the Rust side's commands (src/gen/bindings.ts, generated from
// crates/window by `cargo xtask bindings`), its events, and the opener and
// clipboard plugins. The gateways in src/lib call the services here by
// their Go names with Go's positional arguments; components never import
// this file or the bindings.
//
// Conventions kept from the Wails build (surface.md 7.6): a failed call
// rejects with an Error whose message is the Rust side's text, and an
// event callback gets the payload as `data`.

import { listen } from "@tauri-apps/api/event"
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
  On<T = unknown>(name: string, callback: (ev: HostEvent<T>) => void): () => void {
    let off: (() => void) | null = null
    let cancelled = false
    listen<T>(name, (e) => callback({ name, data: e.payload }))
      .then((unlisten) => {
        if (cancelled) unlisten()
        else off = unlisten
      })
      .catch(() => {})
    return () => {
      cancelled = true
      off?.()
      off = null
    }
  },
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
  AllowAndRelease: (deliveryId: string) => done(commands.inboxAllowAndRelease(deliveryId)),
  Policy: () => call(commands.inboxPolicy()),
  SetPolicy: (p: B.PolicyInput) => done(commands.inboxSetPolicy(p)),
  SetPaused: (paused: boolean) => done(commands.inboxSetPaused(paused)),
}

// Wind down all (card #220): every agent saves its place, hands back its
// cards and stops; Pause all turns on after, if asked.
export const WindDownService = {
  All: (scope: string, pauseAfter: boolean) => call(commands.windDownAll(scope, pauseAfter)),
  Status: () => call(commands.windDownStatus()),
  Stop: (key: string) => done(commands.windDownStop(key)),
}

export const BoardService = {
  Cards: () => call(commands.boardCards()),
  IdleReport: () => call(commands.boardIdleReport()),
  Card: (id: string) => call(commands.boardCard(id)),
  DoneCards: (limit: number, before: string) => call(commands.boardDoneCards(limit, before)),
  DoneCardsIn: (limit: number, before: string, scope: string) =>
    call(commands.boardDoneCardsIn(limit, before, scope)),
  ClearDoneCount: (olderThanSecs: number, scope: string) =>
    call(commands.boardClearDoneCount(olderThanSecs, scope)),
  ClearDone: (olderThanSecs: number, ids: string[], scope: string) =>
    call(commands.boardClearDone(olderThanSecs, ids, scope)),
  Message: (taskId: string, msgId: string) => call(commands.boardMessage(taskId, msgId)),
  Comment: (id: string, text: string, tell: string, mentions: B.Mention[]) =>
    call(commands.boardComment(id, text, tell, mentions)),
  CommentWithFiles: (
    id: string,
    text: string,
    tell: string,
    mentions: B.Mention[],
    files: B.UploadFile[]
  ) => call(commands.boardCommentWithFiles(id, text, tell, mentions, files)),
  Answer: (taskId: string, blockerId: string, option: number, text: string, wake: boolean, mentions: B.Mention[]) =>
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
      commands.boardAnswerWithFiles(taskId, blockerId, option, text, wake, mentions, files)
    ),
  ResolveBlocker: (taskId: string, blockerId: string, note: string, mentions: B.Mention[]) =>
    call(commands.boardResolveBlocker(taskId, blockerId, note, mentions)),
  MentionCandidates: () => call(commands.boardMentionCandidates()),
  AskAgain: (id: string) => done(commands.boardAskAgain(id)),
  AskRepos: () => call(commands.boardAskRepos()),
  CloseAsk: (id: string, note: string) => call(commands.boardCloseAsk(id, note)),
  Close: (id: string, state: string, note: string) => call(commands.boardClose(id, state, note)),
  Delete: (id: string) => done(commands.boardDelete(id)),
  Pin: (id: string, pinned: boolean) => done(commands.boardPin(id, pinned)),
  // Pause / Resume (card #207): who was told.
  Pause: (id: string, paused: boolean, note: string) =>
    call(commands.boardPause(id, paused, note)),
  MovePin: (id: string, before: string) => done(commands.boardMovePin(id, before)),
  Place: (id: string, x: number, y: number) => done(commands.boardPlace(id, x, y)),
  Create: (input: B.NewCardInput) => call(commands.boardCreate(input)),
  StartThread: (text: string, mentions: B.Mention[], boardId: string) =>
    call(commands.boardStartThread(text, mentions, boardId)),
  StartThreadWithFiles: (text: string, mentions: B.Mention[], files: B.UploadFile[]) =>
    call(commands.boardStartThreadWithFiles(text, mentions, files)),
  Threads: (boardId: string) => call(commands.boardThreads(boardId)),
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

export const BoardsService = {
  List: () => call(commands.boardsList()),
  Create: (name: string, tag: string) => call(commands.boardsCreate(name, tag)),
  Rename: (boardId: string, name: string) => call(commands.boardsRename(boardId, name)),
  AddToSpace: (spaceId: string, name: string, tag: string) =>
    call(commands.boardsAddToSpace(spaceId, name, tag)),
  Pin: (boardId: string, pinned: boolean) => done(commands.boardsPin(boardId, pinned)),
  Leave: (boardId: string) => done(commands.boardsLeave(boardId)),
  Forget: (boardId: string) => done(commands.boardsForget(boardId)),
  ForgetNow: (boardId: string) => done(commands.boardsForgetNow(boardId)),
  RemoveMember: (boardId: string, personId: string) =>
    call(commands.boardsRemoveMember(boardId, personId)),
  SetRole: (boardId: string, personId: string, role: string) =>
    call(commands.boardsSetRole(boardId, personId, role)),
  SharePreview: (taskId: string, boardId: string) => call(commands.boardsSharePreview(taskId, boardId)),
  Share: (taskId: string, boardId: string, hash: string) => call(commands.boardsShare(taskId, boardId, hash)),
  Withdraw: (taskId: string) => call(commands.boardsWithdraw(taskId)),
  Settings: (boardId: string) => call(commands.boardsSettings(boardId)),
  SetReceive: (boardId: string, preset: string, people: string[]) =>
    call(commands.boardsSetReceive(boardId, preset, people)),
  SetAutonomy: (boardId: string, preset: string) => call(commands.boardsSetAutonomy(boardId, preset)),
  SetThreads: (boardId: string, mode: string) => call(commands.boardsSetThreads(boardId, mode)),
  SetOpenCards: (boardId: string, to: string) => call(commands.boardsSetOpenCards(boardId, to)),
  SetReviewer: (boardId: string, agent: string, repo: string) =>
    call(commands.boardsSetReviewer(boardId, agent, repo)),
  Invite: (boardId: string) => call(commands.boardsInvite(boardId)),
  JoinInvite: (link: string) => call(commands.boardsJoinInvite(link)),
  ConfirmInviteWords: (words: string) => call(commands.boardsConfirmInviteWords(words)),
  InviteStatus: () => call(commands.boardsInviteStatus()),
  CancelInvite: () => done(commands.boardsCancelInvite()),
}

export const DevicesService = {
  Devices: () => call(commands.devicesDevices()),
  Colors: () => call(commands.devicesColors()),
  SetDeviceColor: (deviceId: string, color: string) => done(commands.devicesSetDeviceColor(deviceId, color)),
  RenameDevice: (deviceId: string, name: string) => call(commands.devicesRenameDevice(deviceId, name)),
  AddDevice: (role: string) => call(commands.devicesAddDevice(role)),
  JoinWithCode: (code: string, server: string) => call(commands.devicesJoinWithCode(code, server)),
  ConfirmWords: (words: string) => done(commands.devicesConfirmWords(words)),
  ConfirmKey: (key: string) => done(commands.devicesConfirmKey(key)),
  PairingStatus: () => call(commands.devicesPairingStatus()),
  CancelPairing: () => done(commands.devicesCancelPairing()),
  RemoveDevice: (deviceId: string) => done(commands.devicesRemoveDevice(deviceId)),
  ResetDevice: () => done(commands.devicesResetDevice()),
  HomeServer: () => call(commands.devicesHomeServer()),
  SetHomeServer: (on: boolean, outside: boolean, publicAddr: string) =>
    call(commands.devicesSetHomeServer(on, outside, publicAddr)),
  SetHomeServerAwake: (on: boolean) => call(commands.devicesSetHomeServerAwake(on)),
  SetPublicRelays: (on: boolean) => call(commands.devicesSetPublicRelays(on)),
  Relay: () => call(commands.devicesRelay()),
  SetRelay: (credentialUrl: string) => call(commands.devicesSetRelay(credentialUrl)),
}

export const SettingsService = {
  GetNotifications: () => call(commands.settingsGetNotifications()),
  SetNotifications: (o: B.NotifyOptions) => call(commands.settingsSetNotifications(o)),
  TestNotification: () => done(commands.settingsTestNotification()),
  GetLaunchAtLogin: () => call(commands.settingsGetLaunchAtLogin()),
  SetLaunchAtLogin: (enabled: boolean) => done(commands.settingsSetLaunchAtLogin(enabled)),
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
