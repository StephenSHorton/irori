// The preview's Trust and start (card #717): outside the app there is no
// host to trust a folder or start an agent, so the button answers with one
// of the outcomes the app gives, picked by ?trust= (held, cap or start;
// cap when it's missing). The app's words come from
// crates/window/src/agents_trust.rs; these copy them for the look.

import type { TrustStarted } from "@/gen/bindings"

export const SAMPLE_TRUST_SAID: Record<string, string> = {
  held: "Trusted. sample-heron already has it, so no new agent starts.",
  cap: "Trusted. It starts when a slot frees (1 of 1 in use).",
  start: "Trusted. Its agent is starting.",
}

export function sampleTrustAndStart(
  folder: string,
  search: string = typeof location === "undefined" ? "" : location.search
): Promise<TrustStarted> {
  const pick = new URLSearchParams(search).get("trust") ?? "cap"
  return Promise.resolve({
    folder,
    already: false,
    said: SAMPLE_TRUST_SAID[pick] ?? SAMPLE_TRUST_SAID.cap,
  })
}
