// The muted line under an assistant's answer or waiting line in
// #office-assistants (card #1062, docs/design/assistants-talk.md §3, "What
// the label proves"): it says plainly that an answer is a reading of the
// space, not the owner's word, and who it replies to.

import type { Member, Post } from "./conversations"

const nameOf = (m: Member | undefined): string => {
  const n = m?.name?.trim()
  return !n ? "someone" : n === "you" ? "you" : n
}

// replyLine is "" for a post that replies to nothing or isn't an
// assistant's. spaceName "" says "this space".
export function replyLine(
  post: Pick<Post, "by" | "replyTo" | "waiting">,
  posts: readonly Pick<Post, "postId" | "by">[],
  members: readonly Member[],
  spaceName: string
): string {
  if (!post.replyTo) return ""
  const by = members.find((m) => m.key === post.by)
  if (!by?.assistant) return ""
  const asker = members.find(
    (m) => m.key === posts.find((p) => p.postId === post.replyTo)?.by
  )
  const to = `replying to ${nameOf(asker)}`
  if (post.waiting) {
    const owner =
      by.owner?.trim() ||
      (by.name ?? "")
        .replace(/^.* · /, "")
        .replace(/'s assistant$/, "")
        .trim() ||
      "its owner"
    return `Waiting on ${owner} · ${to}`
  }
  return `Answered from what is on ${spaceName || "this space"} · ${to}`
}
