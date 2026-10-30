# Spike 01: can a Claude Code mod wake its own session on Windows?

*2026-10-26, Windows 11, Claude Code 2.1.311. Mod: [`spike/mod-wake`](../../spike/mod-wake).*

The plan (research/01 §1) named this the one real risk: waking a Claude Code session depends on the mod API (`$.prompt.submit`) and the wake stream on `$.process.spawn`. Both were proven on Windows in a `claude --bg` session.

## What ran

```
claude --bg --plugin-dir C:/Users/4step/projects/irori/spike/mod-wake "Reply with exactly: hello-spike"
```

The mod, on `session.start`, spawns `node -e 'setTimeout(() => console.log("WAKE"), 4000); …'`, reads its stdout through `$.process.spawn`, and on `WAKE` calls `$.prompt.submit({ text: 'IRORI_SPIKE wake. Reply with exactly: irori-spike-pong' })`. It logs `session.start`, `session.append`, `turn.start` and `turn.complete` to `%TEMP%\irori-spike.log`.

## Results

| Question | Answer |
|---|---|
| Does `$.process.spawn` stream a long-lived child's stdout on Windows? | **Yes.** `spawn stdout: WAKE` arrived about 4 s after start. |
| Does `$.prompt.submit` start a turn in a background (`--bg`) session with nobody attached? | **Yes.** It resolved, `turn.start` fired with the submitted text, and the model replied `irori-spike-pong`. |
| How does the woken turn look to the model? | As a plugin message, not the user: the row has `origin: {kind: "plugin", name: "irori-spike"}` and the text is prefixed "The irori-spike plugin sent a message:", followed by Claude Code's own note that the plugin started the turn in the user's place. Irori's `<irori-message trust=untrusted>` fence sits on top of that. |
| What does `session.append` carry? (research/04 §8 Q1) | Typed rows: `{message: {type, role, content[]}, door, origin, uuid}`. Seen doors: `prompt` (origin `composer` or `plugin`), `response` (origin `model` with the model id), plus tool rows carrying `name`. Enough for capture without tailing JSONL; whether tool rows carry the full `toolUseResult` (structuredPatch) still needs checking in v0.3. |
| Is `turn.start`/`turn.complete` usable for "engaged"/"answered"? | Yes, both fire for composer and plugin turns. |

## Gotchas found (these shape `irori attach` and the real mod)

1. **The `session.start` hook ran twice in one `--bg` session**, about 6 s apart: two children each printed `WAKE`, so the wake was submitted twice. Only one `session.start` log line made it out, so it's not yet clear whether the event fired twice or the module was loaded twice (for example by plugin-dir watching).
   - **Rerun (same day):** with each module instance tagged, a fresh `--bg` run showed one instance, one `session.start`, one child and one wake. The double didn't reproduce. The likeliest cause is a reload: that run started seconds after `register.ts` was edited, and `CLAUDE_CODE_PLUGIN_DIR_WATCH=1` reloads mods whose files change. Edits to an installed mod can reload it mid-session, so the rule stands: `irori attach` dedupes per session id. The real mod must make `irori attach` idempotent per session id (the Core refuses a second attach for the same session, or the mod keys its child on `$.session.id()`), and the delivery state machine must consume each item once regardless.
2. **The mod sandbox has no Node `process` global.** `process.env` / `process.platform` make the module fail to load. Config goes through plugin `userConfig` options; OS facts come from the spawned helper.
3. **The loader statically checks `$`:** it may only be passed to functions declared at the top of the module (a `function` declaration or a top-level `const` bound to one). A `const note = async ($) => …` inside `register` is rejected with "hooks module did not load".
4. **`claude --bg` refuses an untrusted workspace** ("Workspace not trusted. Run `claude` in <dir> once…"). Reviving into a repo that was never opened interactively will fail; the Doctor tab should check trust before offering revive.

## Not covered yet

- macOS (owed; this machine is Windows only).
- A Code-tab (desktop) session, and an interactive terminal session that is mid-turn when the wake arrives (queueing behaviour).
- Channel push (research/04 §8 Q6) and `-p` mode (Q7). The mod path above already covers headless wake on Windows, which was the main reason for Q7.
