// tellTo is who hears about a card, the same rule as coord's recipients()
// (internal/coord/board_impl.go): its agent holders; with no holders at
// all, its creator when that is an agent. A card held only by people has
// no agent to tell. by is the actor, who never hears their own action.
export function tellTo(
  holders: readonly string[] | null | undefined,
  createdBy: string | undefined,
  by = ""
): string[] {
  const hs = holders ?? []
  if (hs.length) return hs.filter((h) => h !== by && !isAgentless(h))
  return createdBy && createdBy !== by && !isAgentless(createdBy)
    ? [createdBy]
    : []
}

// sweeper is coord's own key for actions the board takes itself.
const sweeper = "irori"

function isAgentless(key: string): boolean {
  return key.startsWith("user:") || key === sweeper
}
