// goThere takes the window to what a go line names and highlights it
// (card #784): a page, a card or thread (its page's header), a setting
// (Settings at its section, the control's row) or the top bar's Pause
// agents. A PR opens on GitHub. It resolves "" when it got there, else a
// plain line saying why not, for the chat to show.

import { PaletteJump } from "@/components/command-palette"
import { openSettings } from "@/components/settings/settings-menu"
import { cardRef, cards as cardsApi } from "@/lib/cards"
import { runCommand } from "@/lib/commands"
import { settingSpot, type GoPage, type GoTarget } from "@/lib/goto"
import { go, type Section } from "@/lib/nav"
import { spotlight } from "@/lib/spotlight"
import { errorText } from "@/lib/talk"

export type GoDeps = {
  openCard: (id: string) => void
  openThread: (id: string) => void
  // before runs once the place is found, just before the window moves
  // (on a phone the chat closes then).
  before?: () => void
}

const pageCommands: Partial<Record<GoPage, string>> = {
  home: "nav.home",
  talk: "nav.talk",
  notes: "nav.notes",
  boards: "nav.boards",
  storage: "nav.storage",
  notifications: "nav.notifications",
  held: "nav.held",
}

const pageSections: Partial<Record<GoPage, Section>> = {
  home: { kind: "home" },
  talk: { kind: "talk", id: "" },
  notes: { kind: "notes" },
  boards: { kind: "boards" },
  desk: { kind: "desk", repo: "" },
  storage: { kind: "storage", tab: "" },
  notifications: { kind: "notifications" },
  profile: { kind: "profile" },
}

// jump closes what would hide the place (Settings, Held, the message
// dialog), as a pick in the palette does.
function jump() {
  window.dispatchEvent(new Event(PaletteJump))
}

export async function goThere(t: GoTarget, deps: GoDeps): Promise<string> {
  try {
    switch (t.kind) {
      case "page": {
        deps.before?.()
        jump()
        const id = pageCommands[t.page]
        if (id && runCommand(id)) return ""
        const s = pageSections[t.page]
        if (!s) return "That page can't be opened from here."
        go(s)
        return ""
      }
      case "card": {
        const r = await cardRef(t.ref)
        if (!r) return `Couldn't find ${t.ref}.`
        deps.before?.()
        jump()
        if (r.thread) deps.openThread(r.id)
        else deps.openCard(r.id)
        void spotlight(
          `[data-card-page="${CSS.escape(r.id)}"] [data-card-head]`
        )
        return ""
      }
      case "setting": {
        const s = settingSpot(t.key)
        if (!s) return "That setting isn't one the chat knows."
        deps.before?.()
        if (!s.section) {
          // The top bar's Pause agents: a phone's open conversation hides
          // the top bar, so Home first.
          jump()
          const sel = `[data-spot="${CSS.escape(s.key)}"]`
          if (!document.querySelector(sel) && !runCommand("nav.home"))
            go({ kind: "home" })
          void spotlight(sel)
          return ""
        }
        openSettings(s.section)
        if (s.key !== s.section)
          void spotlight(`[data-setting="${CSS.escape(s.key)}"]`)
        return ""
      }
      case "pr":
        deps.before?.()
        await cardsApi.openURL(t.url)
        return ""
    }
  } catch (err) {
    return errorText(err)
  }
}
