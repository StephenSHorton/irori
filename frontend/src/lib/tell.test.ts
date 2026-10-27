import { describe, expect, test } from "bun:test"

import { tellTo } from "./tell"

// Mirrors coord's recipients() table: a person's comment reaches these.
describe("tellTo", () => {
  const cases: {
    name: string
    holders: string[] | null
    createdBy: string
    want: string[]
  }[] = [
    {
      name: "agent holders",
      holders: ["a", "b"],
      createdBy: "c",
      want: ["a", "b"],
    },
    {
      name: "people skipped",
      holders: ["user:dev-a", "a"],
      createdBy: "c",
      want: ["a"],
    },
    {
      name: "no holders: agent creator",
      holders: [],
      createdBy: "c",
      want: ["c"],
    },
    {
      name: "null holders: agent creator",
      holders: null,
      createdBy: "c",
      want: ["c"],
    },
    {
      name: "no holders: person creator",
      holders: [],
      createdBy: "user:dev-a",
      want: [],
    },
    {
      name: "agent assigned it to the person: nobody",
      holders: ["user:dev-a"],
      createdBy: "c",
      want: [],
    },
    {
      name: "sweeper is not an agent",
      holders: ["irori"],
      createdBy: "c",
      want: [],
    },
  ]
  for (const c of cases) {
    test(c.name, () => {
      expect(tellTo(c.holders, c.createdBy, "user:dev-a")).toEqual(c.want)
    })
  }
})
