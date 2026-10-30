// The window's models: the generated binding types (src/gen/bindings.ts,
// from crates/window) with a factory per object type (src/gen/models.ts),
// and the Go string enums as constant objects under their Go names, so
// code reads TaskState.Working as it did with the Wails bindings.
//
// Only the samples and tests call the factories; the gateways pass the
// host's JSON through, which always carries every field a type requires.

import type * as B from "../gen/bindings"

export * from "../gen/models"

/** coord.TaskState. */
export const TaskState = {
  $zero: "",
  Submitted: "submitted",
  Working: "working",
  InputRequired: "input_required",
  Completed: "completed",
  Failed: "failed",
  Canceled: "canceled",
  Blocked: "blocked",
} as const satisfies Record<string, B.TaskState | "">
export type TaskState = B.TaskState

/** The holder line's state (cards_service.go Holding). */
export const Holding = {
  $zero: "",
  HoldNobody: "nobody",
  HoldPerson: "person",
  HoldWaiting: "waiting",
  HoldAssigned: "assigned",
  HoldActive: "active",
  HoldIdle: "idle",
  HoldOffline: "offline",
  HoldGone: "gone",
} as const satisfies Record<string, B.Holding | "">
export type Holding = B.Holding

/** coord.LinkKind. */
export const LinkKind = {
  $zero: "",
  LinkURL: "url",
  LinkPath: "path",
  LinkCommit: "commit",
} as const satisfies Record<string, B.LinkKind | "">
export type LinkKind = B.LinkKind

/** coord.BlockerKind. */
export const BlockerKind = {
  $zero: "",
  BlockedOnLease: "lease",
  BlockedOnTask: "task",
  BlockedOnPerson: "person",
  BlockedOnAgent: "agent",
  BlockedOnExternal: "external",
} as const satisfies Record<string, B.BlockerKind | "">
export type BlockerKind = B.BlockerKind

/** inbox.State. */
export const State = {
  $zero: "",
  Queued: "queued",
  Held: "held",
  Delivered: "delivered",
  Engaged: "engaged",
  Answered: "answered",
  Deferred: "deferred",
  Skipped: "skipped",
  Rejected: "rejected",
  Dismissed: "dismissed",
  Moved: "moved",
} as const satisfies Record<string, B.State | "">
export type State = B.State

/** inbox.HoldReason. */
export const HoldReason = {
  $zero: "",
  HoldBlocked: "blocked",
  HoldNotAllowed: "not_allowed",
  HoldBypassReceiver: "bypass_receiver",
  HoldPaused: "paused",
  HoldHopLimit: "hop_limit",
  HoldUnowned: "unowned",
  HoldMemberWake: "member_wake",
  HoldCrossPerson: "cross_person",
  HoldMemberLease: "member_lease",
} as const satisfies Record<string, B.HoldReason | "">
export type HoldReason = B.HoldReason

/** inbox.Mode (the wake policy). */
export const Mode = {
  $zero: "",
  Everyone: "everyone",
  Allowlist: "allowlist",
  Open: "open",
} as const satisfies Record<string, B.Mode | "">
export type Mode = B.Mode

/** inbox.DeliveryMode. */
export const DeliveryMode = {
  $zero: "",
  ModeWake: "wake",
  ModeNextTurn: "next_turn",
} as const satisfies Record<string, B.DeliveryMode | "">
export type DeliveryMode = B.DeliveryMode

/** inbox.AddressKind. */
export const AddressKind = {
  $zero: "",
  ToAgent: "agent",
  ToRepo: "repo",
  ToReply: "reply",
  ToPR: "pr",
} as const satisfies Record<string, B.AddressKind | "">
export type AddressKind = B.AddressKind

/** inbox.Trust. */
export const Trust = {
  $zero: "",
  Untrusted: "untrusted",
  UserTrust: "user",
} as const satisfies Record<string, B.Trust | "">
export type Trust = B.Trust

/** presence.ChangeKind. */
export const ChangeKind = {
  $zero: "",
  Added: "added",
  Updated: "updated",
  Removed: "removed",
} as const satisfies Record<string, B.ChangeKind | "">
export type ChangeKind = B.ChangeKind
