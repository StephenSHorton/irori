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
import { writeText } from "@tauri-apps/plugin-clipboard-manager"
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
}

// ---- services ---------------------------------------------------------------------

export const RosterService = {
  Agents: () => call(commands.rosterAgents()),
  Device: () => call(commands.rosterDevice()),
  Version: () => call(commands.rosterVersion()),
  Recent: (limit: number) => call(commands.rosterRecent(limit)),
  Gone: (limit: number) => call(commands.rosterGone(limit)),
  Revive: (sessionId: string) => call(commands.rosterRevive(sessionId)),
}

export const InboxService = {
  List: (limit: number) => call(commands.inboxList(limit)),
  Held: () => call(commands.inboxHeld()),
  HeldHistory: (limit: number) => call(commands.inboxHeldHistory(limit)),
  Counts: () => call(commands.inboxCounts()),
  Get: (deliveryId: string) => call(commands.inboxGet(deliveryId)),
  Send: (to: string, repo: string, text: string) => call(commands.inboxSend(to, repo, text)),
  Reply: (deliveryId: string, text: string) => done(commands.inboxReply(deliveryId, text)),
  MarkRead: (deliveryId: string) => done(commands.inboxMarkRead(deliveryId)),
  Release: (deliveryId: string) => done(commands.inboxRelease(deliveryId)),
  Dismiss: (deliveryId: string) => done(commands.inboxDismiss(deliveryId)),
  AllowAndRelease: (deliveryId: string) => done(commands.inboxAllowAndRelease(deliveryId)),
  Policy: () => call(commands.inboxPolicy()),
  SetPolicy: (p: B.PolicyInput) => done(commands.inboxSetPolicy(p)),
  SetPaused: (paused: boolean) => done(commands.inboxSetPaused(paused)),
}

export const BoardService = {
  Cards: () => call(commands.boardCards()),
  Card: (id: string) => call(commands.boardCard(id)),
  DoneCards: (limit: number, before: string) => call(commands.boardDoneCards(limit, before)),
  Message: (taskId: string, msgId: string) => call(commands.boardMessage(taskId, msgId)),
  Comment: (id: string, text: string, tell: string) => call(commands.boardComment(id, text, tell)),
  Answer: (taskId: string, blockerId: string, option: number, text: string, wake: boolean) =>
    call(commands.boardAnswer(taskId, blockerId, option, text, wake)),
  ResolveBlocker: (taskId: string, blockerId: string, note: string) =>
    call(commands.boardResolveBlocker(taskId, blockerId, note)),
  Ask: (text: string, repoKey: string, wake: boolean) => call(commands.boardAsk(text, repoKey, wake)),
  AskWithFiles: (text: string, repoKey: string, wake: boolean, files: B.UploadFile[]) =>
    call(commands.boardAskWithFiles(text, repoKey, wake, files)),
  AskAgain: (id: string) => done(commands.boardAskAgain(id)),
  AskRepos: () => call(commands.boardAskRepos()),
  CloseAsk: (id: string, note: string) => call(commands.boardCloseAsk(id, note)),
  Close: (id: string, state: string, note: string) => call(commands.boardClose(id, state, note)),
  Delete: (id: string) => done(commands.boardDelete(id)),
  Pin: (id: string, pinned: boolean) => done(commands.boardPin(id, pinned)),
  MovePin: (id: string, before: string) => done(commands.boardMovePin(id, before)),
  Reopen: (id: string) => call(commands.boardReopen(id)),
}

export const BoardsService = {
  List: () => call(commands.boardsList()),
  Create: (name: string, tag: string) => call(commands.boardsCreate(name, tag)),
  Rename: (boardId: string, name: string) => call(commands.boardsRename(boardId, name)),
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
  Relay: () => call(commands.devicesRelay()),
  SetRelay: (credentialUrl: string) => call(commands.devicesSetRelay(credentialUrl)),
}

export const SettingsService = {
  GetNotifications: () => call(commands.settingsGetNotifications()),
  SetNotifications: (o: B.NotifyOptions) => call(commands.settingsSetNotifications(o)),
  TestNotification: () => done(commands.settingsTestNotification()),
  GetLaunchAtLogin: () => call(commands.settingsGetLaunchAtLogin()),
  SetLaunchAtLogin: (enabled: boolean) => done(commands.settingsSetLaunchAtLogin(enabled)),
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
