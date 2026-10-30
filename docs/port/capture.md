# Port spec: capture area

Scope: `internal/capture` (with `capture/transcript`, `capture/gitlink`), `internal/attach` (+ `attach/runner`, top-level `attach.go`), `internal/proc`, `internal/repo`, `internal/gitsha`, `internal/fswatch`, `internal/blob`, `internal/scrub`, `internal/search`.
All `file:line` refs are relative to `../irori-go/` (Go be67a86, the source of truth, read-only).
"Go" = Go 1.26 semantics (`go.mod:7` says `go 1.26.2`; toolchain seen: go1.26.5).

---

## 0. Byte-identity invariants (get these wrong and replay double-counts or misses history)

| # | Invariant | Where |
|---|---|---|
| I1 | Row ids: `prefix + hex(sha256(parts.join("\|")))[..20]` (20 lowercase hex chars) | `internal/capture/service.go:421-425` |
| I1a | Turn row id: `t-` over `key\|turnId`; over `key\|agentId\|turnId` only when agentId != "" | `service.go:429-434` |
| I1b | Edit row id: `e-` over `key\|toolUseId\|e.Path` where `e.Path` is the path **exactly as the transcript wrote it** (not the cleaned absolute path computed right after) | `internal/capture/rows.go:182`, `service.go:436-438` |
| I1c | Summary row id: `s-` over `key\|agentId\|kind\|X`, X = rowUuid, else the **scrubbed** text (agentId may be "" and still takes its slot) | `rows.go:229-236`, `service.go:440-442` |
| I1d | In-memory dedupe ids (never logged): `c\|key\|toolUseId` (commit cmd), `p\|ledgerKey\|session\|role` (PR), commit key `repoKey\|lower(sha)` | `service.go:444-448` |
| I2 | Agent key = `<deviceId>/<sessionId>`; deviceId is 32 hex in real data | `internal/capture/tailer.go:246` |
| I3 | Cursor file `<DataDir>/capture-cursors.json` = `{"files":{path:int64},"reflogs":{normRoot:int64}}`. `files` keys = `filepath.Join(ClaudeDir,"projects",slug,name)` with native separators and on-disk case (Windows, e.g. `C:\Users\pat\.claude\projects\<slug>\<sid>.jsonl`). `reflogs` keys = `normRoot(root)` = Clean + native seps + **lowercased on Windows** (e.g. `c:\users\pat\projects\my-app`). A different spelling makes the file "new": backfill rules re-run (safe through ids, but a full re-read / skipped history) | `internal/capture/cursors.go:12-29`, `tailer.go:125-171`, `internal/capture/roots.go:92-98`, `internal/capture/repos.go:93,122` |
| I4 | Blob id = sha256 hex of the stored bytes (after truncation), file `<DataDir>/blobs/<id[0:2]>/<id>`, gzip, content hash verified on read | `internal/blob/blob.go:1-6`, `internal/blob/store.go:66-104,146-184` |
| I5 | `gitsha.Blob` = SHA-1 of `"blob " + decimal len + "\0" + data`; `Pair` = (Blob(data), Blob(CRLF→LF) or "" if no CRLF) | `internal/gitsha/gitsha.go:15-39` |
| I6 | Patch text: `--- a/<p>\n+++ b/<p>\n` then per hunk `@@ -oldStart,oldLines +newStart,newLines @@\n` and each line + `\n`; p is cwd-relative when under cwd, forward slashes | `internal/capture/transcript/target.go:113-136`, `internal/capture/transcript/result.go:242-246` |
| I7 | Scrub marker `[redacted:<kind>]` + every `\r`/`\n` of the match (line count preserved) | `internal/scrub/scrubber.go:131-146` |
| I8 | Repo keys `remote:<host>/<path>` or `root:<smallest root sha>` | `internal/repo/repo.go:1-8`, `internal/repo/resolver.go:95-132` |

---

## 1. What the area does; public interface

**capture** tails Claude Code transcripts (`~/.claude/projects/**.jsonl`) into log rows (turns, edits, summaries, commit commands, PR ledger rows), watches each known worktree's `logs/HEAD` reflog, imports commits and links them to edits, keeps decisions, and owns the search index and Show renderings. Its state is a fold over the event log rebuilt at `New` (`internal/capture/capture.go:1-12`, `service.go:174-212`).

Log kinds written and replayed (`capture.go:31-41`, replay list `service.go:43-46`):

| kind | payload | actor |
|---|---|---|
| `capture.turn` | TurnRow | `agent:<key>` (`service.go:454`, `rows.go:172`) |
| `capture.edit` | EditRow | `agent:<key>` |
| `capture.summary` | SummaryRow | `agent:<key>` |
| `capture.commitcmd` | CommitCmdRow | `agent:<key>` |
| `pr.ledger` | ledger.Entry | `agent:<key>` from Bash; `irori` from pr-link rows (`rows.go:294,315`) |
| `repo.seen` | `{repoKey, root, at}` | `irori` (`rows.go:106`, `service.go:49-53`) |
| `commit.seen` | CommitRow | `irori` (`repos.go:250`) |
| `commit.linked` | CommitLink; newest per sha wins | `irori` (`repos.go:342,472,592`) |
| `decision.recorded` | decision.Decision | `user` or `agent:<key>` (`internal/capture/why.go:293-297`) |

Real counts on Stephen's PC (2026-10-29): commitcmd 1054, edit 2948, summary 231, turn 1986, commit.linked 1424, commit.seen 1099, pr.ledger 90, repo.seen 129.

Public methods (`capture.go:315-378`, `internal/capture/search.go:100-162`, `internal/capture/overview.go:86`): `New(ctx, store, roster, Options)`, `Run(ctx)`, `Timeline(TimelineQuery)`, `Edits(EditQuery)`, `Commits(repoKey, limit)`, `Commit(sha)`, `Repos()`, `Blob(id)`, `PROwners(prKey)`, `PRs(repo)`, `Why(ctx, WhyQuery)`, `RecordDecision(ctx, DecisionInput)`, `Decisions(Filter)`, `Subscribe(ctx) -> chan Change` (buffer 64, full = drop, `service.go:385-409`), `Status()`, `AddSource(search.Source)` (panics on own/duplicate kind), `Search`, `Show`, `Overview`.

Inputs from other areas: `Roster.Agents() []presence.AgentCard` (uses `Status`, `SessionID`, `Cwd`, `Key`, `Name`, `Title`, `RepoKey`; "gone" ignored, `tailer.go:103-114`); `store.Store` (`Append`, `List` with `Kinds/After/Limit=MaxListLimit` paging by `(hlc,device,seq)` cursor, `ev.HLC.Wall()`, `service.go:186-201,325`); `ledger` (`FromBash`, `Key`, `RepoKey`, `RepoFromKey`, `Ledger.Add/ForSession/Owners/ForRepo`, roles author/touched/reviewer); `decision` (View, Validate, MakeAnchor, Check, ParseLines, CanSupersede). Core wiring: `internal/core/core.go:229-238` (Options `ClaudeDir, DataDir, DeviceID, RepoKey: repo.Resolver.Key, Scrub: core scrubber`; `AddSource(coord)`, `AddSource(inbox)`); core calls `scrubber.SetExact(secrets...)` (`core.go:650`).

**attach**: `irori attach --session <id> [--cwd …] [--mod-version …]`, spawned by the mod as `[path,'attach','--session',sid,'--cwd',cwd,'--mod-version',MOD_VERSION]` (`mod/hooks/register.tsx` ~411; `MOD_VERSION='0.4.1'` `mod/hooks/lib.ts:7`). Copies NDJSON events from the Core to stdout (section 2.9).

**proc**: `Alive(pid)`, `StartTime(pid)`, `WaitExit(ctx,pid,startedBy)`, `FiletimeTicks(t)` (Windows). Callers: attach parent-watch, discovery lockfile liveness (`internal/discovery/lockfile.go:99`), presence liveness (`internal/presence/live_windows.go:11-29`, `live_darwin.go:50-66`).

**repo**: `Resolver.Key(dir)`, `NormalizeRemote(url)`.
**gitsha**: `Blob`, `LF`, `Pair`.
**fswatch**: `Watch(ctx, root, recursive, debounce) -> Watcher{Events(), Close()}`, `ErrUnsupported`.
**blob**: `Store` (patches/summaries) and `Files` (attachments), `CheckAttachPath`, `SafeName`, `MarkDownloaded`, `Within`, `ValidID`, `SameContent`, `FileRef`.
**scrub**: `New(exact...)`, `SetExact`, `String`, `SkipFile`, `Strict`, `StrictDropped`, `PathMarker`.
**search**: shared types `Kind`, `Ref`, `ParseRef`, `Doc`, `Query`, `Hit`, `ShowOptions`, `Shown`, `Source`, `Page`, `CanRead` (`internal/search/search.go`).

---

## 2. Data formats

### 2.1 Transcript files (input; Claude Code 2.1.29x–2.1.31x)

Paths (`internal/capture/transcript/source.go:9-49`, `tailer.go:123-172`):
- main: `<ClaudeDir>/projects/<slug>/<sessionId>.jsonl`
- subagent: `<ClaudeDir>/projects/<slug>/<sessionId>/subagents/**/agent-<agentId>.jsonl` (walked recursively)
- ids must match `[A-Za-z0-9_-]+` (`source.go:51-63`); segments `""`, `.`, `..` rejected; both slash styles; prefix compared case-insensitively when either side is drive-letter/UNC (`source.go:73-87`).
- ClaudeDir = `CLAUDE_CONFIG_DIR` else `~/.claude` (`internal/paths/*.go` ClaudeDir); DataDir = `IRORI_HOME`, else Windows `%LOCALAPPDATA%\irori`, else `~/.irori`.

Line handling: strip trailing `\r\n` (`internal/capture/transcript/parser.go:86`); skip blank lines and lines whose bytes after leading spaces/tabs start with `{"type":"api-request` (`source.go:89-98`); invalid JSON ignored; rows whose `agentId` != the file's agentId ignored (`parser.go:94`).

Fields read (`parser.go:10-46`, `result.go:10-54`): `type, subtype, uuid, promptId, timestamp (RFC3339Nano), cwd, gitBranch, agentId, isMeta, isCompactSummary, promptSource, origin (object {kind,name} or string), message.content (string|blocks), toolUseResult, content (system), aiTitle, customTitle, prNumber (number or string, "#" trimmed, >0), prUrl, prRepository`. Blocks: `type, text, id, name, input, tool_use_id, is_error, content`. toolUseResult object keys: `type, filePath, notebook_path, oldString, newString, replaceAll, edits[{old_string|oldString,new_string|newString,replace_all|replaceAll}], originalFile (string|null), content, structuredPatch[{oldStart,oldLines,newStart,newLines,lines[]}], staged, error, stdout, bashEditDiff{files[{filePath,hunks[]}],changedFiles[]}, gitOperation (object only), cell_id, stderr`.

Record format notes as documented: `internal/capture/transcript/transcript.go:6-36`.

### 2.2 Parser rules (`transcript.go:188-205`, `parser.go`, `result.go`)

Caps (runes): MaxPrompt 2000, MaxReply 1000, MaxTarget 160, MaxToolsShown 200, MaxStdout 4096 **bytes** (no rune split), MaxSummary 2000 (`transcript.go:47-54`).

- `user` row with any `tool_result` block or a non-null `toolUseResult`: a result row, never a prompt. With toolUseResult but no tool_result block: dropped. Each tool_result block is handled; the row's toolUseResult goes to the **first** block only (`parser.go:302-327`). Then `touch`; an interrupt text block marks the open turn Interrupted.
- `isCompactSummary` → summary `compact_summary` (rowUuid=uuid, at=row time) (`parser.go:328-331`).
- `isMeta` → ignored. `<local-command-stdout>`/`<local-command-stderr>` prefix → activity only (`parser.go:252-256,335-339`).
- Text `[Request interrupted by user]` or `[Request interrupted by user for tool use]` (trimmed) → mark open turn Interrupted, start nothing (`parser.go:258-261,340-346`).
- Prompt: id = promptId else uuid. Same id as open turn → continues (touch). Id already seen (turn closed) → id = uuid (`parser.go:347-360`). Origin (`parser.go:263-292`): origin object kind (name only when kind=="plugin"), or origin string; else `promptSource=="sdk"`→`sdk`; text starts `<command-name>`→`command`; starts `Another Claude session sent a message` or `<teammate-message`→`peer`; else `human`. In a subagent file, `human` with no origin field → `agent` (`parser.go:362-366`). Opening a new prompt closes the open turn with EndedAt = open turn's last activity (or this row's time if zero), reason `next`.
- Prompt text = `text` blocks joined with `\n`; a string content is one text block (`parser.go:205-243`). If decoding blocks as an array fails, each element is decoded alone and bad ones dropped (`parser.go:218-231`).
- `assistant`: non-blank text block → Reply = clip(text, 1000) (last wins). `tool_use`: implicit turn if none open (origin `unknown`, id = row uuid, `parser.go:197-203`); append ToolCall{id,name,target,at} while < 200, else ToolsMore++; remember call by id (`parser.go:377-406`).
- `system`: `turn_duration` closes open turn at row time (`turn_duration`); `away_summary`/`compact_boundary` → summary with `content` as string (`parser.go:408-417`).
- `ai-title`/`custom-title` → summary `ai_title`/`custom_title`, zero time (`parser.go:108-111`); service stamps with parser LastActivity, else now (`rows.go:241-247`).
- `pr-link` → PRLink (`parser.go:443-475`); service writes ledger entry role `touched`, source `pr-link` (`rows.go:301-316`).
- Summary Full = whole text when Text was clipped (`parser.go:428-441`).
- Result (`result.go:118-208`): call looked up by tool_use_id and removed; unknown call with no open turn → implicit turn. The ToolCall's Done/IsError are set only when its turn is still the open one. Unknown tool inferred (`result.go:100-114`): `oldString` key or (`edits`&&`filePath`) → Edit; type create|update + content + filePath → Write; notebook_path+cell_id → NotebookEdit; stdout+stderr → Bash.
  - Edit (not error, not staged): Path = filePath else input.file_path; Fidelity exact; patch from structuredPatch; Before = originalFile; After = apply steps (`edits` else single old/new) with `replace` once or all; any step whose old is empty or absent → After unknown (`result.go:257-287`).
  - Write: Create = type=="create"; Before = originalFile unless create; After = content; a create with no hunks gets an all-`+` hunk (`0,0 → 1,n`, trailing `\n` trimmed, `\r` stripped per line) (`result.go:289-321`).
  - NotebookEdit: needs no `error`; path = notebook_path, filePath, input.notebook_path (`result.go:323-331`).
  - Bash/PowerShell with bashEditDiff: one heuristic edit per unique (pathKey) `changedFiles` entry, patch from the matching `files[].hunks` (`result.go:333-354`).
  - Every Bash/PowerShell result emits BashRun{command=input.command, stdout clipped 4096 bytes from object stdout / string result / block text, gitOperation, isError, at=result time, startedAt=tool_use time} (`result.go:178-207`).
- EditPaths: appended once per pathKey (clean slashes, lowercase if windowsish) while the turn is open (`result.go:219-240`).
- Emission order: a closed Turn comes after that turn's edits and bash runs (`transcript.go:188-192`).
- `clip(s,n)`: runes; cut at n-1, backing up to whitespace within the last n/2 runes (never to index 0), append `…` (`target.go:78-99`). `clipRunes` (service) does not back up: `r[:n-1]+"…"` (`rows.go:327-334`).
- Target (`target.go:30-50`): Read/Edit/Write → file_path; NotebookEdit → notebook_path (cwd-relative with `/` when under cwd, case-insensitive for windowsish); Bash/PowerShell → first non-blank line; Grep/Glob → pattern; WebFetch → url; WebSearch → query; Agent/Task → description; clipped 160.

### 2.3 Logged rows (JSON field names, order as Go marshals)

TurnRow (`capture.go:89-101`) flattens `transcript.Turn` (`transcript.go:83-100`). The row's `id` shadows Turn.`id`; Turn.ID travels as `turnId` and is restored on replay (`service.go:240-242`). Real key order: `id, key, [repoKey], [root], turnId, sessionId, [agentId], origin, [originName], prompt, [reply], [cwd], [branch], startedAt, endedAt, [interrupted], [tools], [toolsMore], [editPaths], [endReason]`. ToolCall: `id,name,[target],[isError],[done],at`. Service post-processing: prompt/reply/targets scrubbed, EditPaths made root-relative with `/` when under Root, noise-dropped paths removed (`rows.go:134-179`).

EditRow (`capture.go:104-123`): `id,key,[agentId],turnId,toolUseId,tool,[repoKey],[root],path,[create],[beforeBlob],[afterBlob],[afterBlobLF],[patch],adds,dels,fidelity,at`. `path` repo-relative `/` inside a repo, else cleaned absolute native path. `patch` = blob id of the **scrubbed** diff, omitted when `scrub.SkipFile(abs)` (`rows.go:181-227`). Before/After content is never logged; only git blob ids.

SummaryRow (`capture.go:126-131`): `id,key,sessionId,[agentId],kind,text(scrubbed),[rowUuid],at,[fullBlob]`.

CommitCmdRow (`capture.go:134-146`): `key,[agentId],turnId,toolUseId,cwd,command (scrubbed first line, ≤300 runes),at,[startedAt],[dir]`. Old rows lack `startedAt`/`dir` (seen in real data); `ref()` falls back to `commandDir(command,cwd)` (`internal/capture/cmddir.go:194-200`). Real `dir` is **not cleaned**: `"C:\\Users\\pat/projects/irori-rust"` (home + raw rest, `cmddir.go:75-89`).

CommitRow (`capture.go:149-155`) flattens `gitlink.Commit` (`internal/capture/gitlink/gitlink.go:66-82`): `repoKey,root,op,[branch],sha,[parents],at,author,subject(scrubbed, ≤200 runes),files[{path,blob,status}],[filesOmitted]`. `at` is `time.Unix(ct,0)` → **local zone offset** of the importing machine (real: `2026-10-29T02:57:49-06:00`).

CommitLink (`capture.go:164-172`): `repoKey,sha,[committer{key,turnId}],files[{path,link,[edits]}],[sessions],[turns],at`. `files` always an array (Classify makes `[]`). Links: exact/partial/heuristic/unattributed (`gitlink.go:105-110`).

repo.seen: `{repoKey,root,at}`; root native, case as git printed (cleaned).

### 2.4 JSON/time encoding rules (Go encoding/json v1)

- time.Time = RFC3339Nano with trailing fractional zeros trimmed (`.36Z`, `.0683527-06:00`), zone as the value carries: transcript times keep `Z`; `now()` times are local offset with Windows 100 ns precision (seen `2026-10-29T09:37:47.0683527-06:00`).
- `omitempty` on a time.Time **does not omit**: zero is written `"0001-01-01T00:00:00Z"` (verified with go1.26.5). No stored capture row has one today, but readers must accept both absent and the zero literal. `Line.At` uses `omitzero` (omitted) (`overview.go:35`).
- Readers: Go matches JSON keys case-insensitively and lets the last duplicate win; any type mismatch on a known field makes `json.Unmarshal` return an error → the whole transcript line is dropped (`parser.go:91-93`). `decodeResult` instead reads `map[string]RawMessage` (exact keys) and decodes field by field ignoring errors (`result.go:56-96`).
- Go replaces invalid UTF-8 and lone surrogate escapes with U+FFFD when decoding; serde_json **errors** on both.
- Go marshals with HTML escaping (`<`→`\u003c`, `>`, `&`, U+2028/9 escaped). Whether Rust-written rows must match Go's encoder byte-for-byte is a store/signing question (see `docs/port/store.md`), not decided here.

### 2.5 Reflog and git

- GitDir: `<root>/.git` if a dir; else first line of the `.git` file (BOM stripped) `gitdir: <p>`, relative to root (`internal/capture/gitlink/reflog.go:15-44`).
- logs/HEAD line: `<old> <new> <name> <<email>> <unix> <±hhmm>\t<msg>` (no tab when msg empty); old/new 40 or 64 hex, same length; tz exactly 5 chars, mm ≤ 59; lowercased shas; At in FixedZone(tz) (`reflog.go:46-130`).
- NewCommits: skip if New zero or seen; `git rev-list --reverse --max-count=200 <old>..<new> --` then fallback `<new>` if that fails; if ≥200 listed, `rev-list --count <rng> --` for `skipped` (`reflog.go:149-198`).
- Describe: `git -c log.showSignature=false show -s --no-color --format=%H%x00%P%x00%ct%x00%an%x00%s <sha> --` (SplitN on NUL into 5), then `git diff-tree -r --no-commit-id --no-renames --raw --no-abbrev --diff-merges=first-parent -z [--root] <sha> --`; parse `:<m> <m> <old> <new> <status>` NUL `<path>` NUL; status = first letter; >500 files → FilesOmitted (`internal/capture/gitlink/describe.go:15-106`). Subject ≤200 runes (no ellipsis).
- Exec: `git --no-pager [-C dir] args…`, env minus `GIT_DIR, GIT_WORK_TREE, GIT_COMMON_DIR, GIT_INDEX_FILE, GIT_OBJECT_DIRECTORY, GIT_ALTERNATE_OBJECT_DIRECTORIES, GIT_CEILING_DIRECTORIES, GIT_DISCOVERY_ACROSS_FILESYSTEM, GIT_NAMESPACE` (case-insensitive names on Windows) plus `GIT_TERMINAL_PROMPT=0 GIT_OPTIONAL_LOCKS=0 GIT_PAGER=cat PAGER=cat`; WaitDelay 500 ms; Windows `CREATE_NO_WINDOW` (0x08000000) + HideWindow; error = `git <args[0]>: <err>: <first stderr line>`; ctx error returned bare (`internal/capture/gitlink/exec.go:13-75`, `exec_windows.go`). repo's runner is the same minus PAGER vars and with stdout trimmed (`internal/repo/resolver.go:164-203`).
- Other git calls: `rev-parse --show-toplevel` (root cache), `rev-parse --abbrev-ref HEAD` (branch; `HEAD`/error → none), `-c core.quotePath=false check-ignore -- <paths…>` (exit 1 = none ignored), `blame -L n,n --porcelain -- <rel>`, `rev-parse HEAD`, `remote get-url origin`, `rev-list --max-parents=0 HEAD`.

### 2.6 Blob store (patches/summaries) and attachment Files

| | Store (`blob/store.go`) | Files (`blob/files.go`) |
|---|---|---|
| dir | `<DataDir>/blobs` (`service.go:146`) | `<DataDir>/files` (`internal/core/core.go:249`), plus `files/partial/` |
| path | `<id[:2]>/<id>` | `<id[:2]>/<id>`; partial `partial/<id>`; put temp `partial/put-*` |
| content | gzip of data; data truncated at 4 MiB (`MaxSize=4<<20`) back ≤3 bytes to a rune start, + `"\n[irori: truncated]\n"` (`blob.go:13-18`, `store.go:32-50`) | raw bytes, cap 256 MiB (`files.go:31`), no scrub |
| id | sha256(stored bytes) | sha256(file) |
| write | temp `<id>.tmp-*` in shard dir, gzip, fsync, close, `place` | copy+hash+512-byte head, fsync, place |
| place | loop: if final is a regular file → success; rename; retry with 5 ms doubling up to ≥1 s; never replace an existing final (`store.go:113-127`) | same |
| read | `openRetry` (Windows sharing violation 32 / access denied 5, 1 ms doubling to 500 ms); must be regular; gunzip ≤ MaxSize+marker; verify sha (`store.go:129-184`) | Open: regular file only |
| ids | 64 lowercase hex only, else ErrBadID without touching FS (`store.go:52-64`) | same |

Dirs created 0700, files 0600 (`store.go:26`, `files.go:55,164,226`). Files API: `PutFile`, `Partial(id)` (O_CREATE|O_RDWR, seek end = resume offset), `Finish(id,size)` (short → keep for resume; wrong/long → delete; ok → place), `DropPartial`, `CopyOut(id,dst)` (O_EXCL, verify hash, remove on error), `SameContent`, `Describe()` text: `<SafeName> (<size>[, <mime>]) sha256=<id>[ from device <first 8>]` with size `%.1f MiB`/`%.1f KiB`/`%d bytes` (`files.go:279-307`). Mime: `mime.TypeByExtension(lower ext)` else `http.DetectContentType(first 512)` (`files.go:271-277`).

### 2.7 Search index (in-memory; nothing on disk)

SQLite `:memory:` on one pinned connection (`internal/capture/index.go:20-44`):
```sql
CREATE VIRTUAL TABLE docs USING fts5(
  ref UNINDEXED, kind UNINDEXED, at UNINDEXED, agent UNINDEXED, repo UNINDEXED, readers UNINDEXED,
  title, body, tokenize = 'porter unicode61')
```
Upsert by ref: map ref→rowid, DELETE by rowid then INSERT with explicit rowid (monotonic counter) (`index.go:73-95`). `at` = UnixNano (0 for zero). `title` = clipRunes(oneLine(title),120). `readers` = `"\n"+join(keys,"\n")+"\n"` or "" (`index.go:97-108`). Query (`index.go:120-190`): text → each `strings.Fields` word with a letter/digit quoted (`"` doubled), joined ` AND `, last gets `*`; text with no usable words → no hits. With text: `snippet(docs,-1,'[',']','…',24)`, order `bm25(docs,0,0,0,0,0,4.0,1.0), at DESC, rowid DESC`; without: `substr(body,1,2000)`, order `at DESC, rowid DESC`. Filters `kind IN`, `repo=`, `agent=`, `at < before`, viewer `(readers='' OR instr(readers, "\n"+viewer+"\n")>0)`; `LIMIT ? OFFSET ?`. Snippet → clipRunes(oneLine,240). Search clamps Limit to default 10, max 51 (`search.go:128-138`). Replay indexes inside one transaction (`service.go:178-183`).

Docs capture indexes (`search.go:220-380`): turn (title = first line of prompt with fenced `<irori-message>` blocks replaced for everyone; body prompt+`\n`+reply), edit (`"<tool> <path> +a -d"`, body path), commit (title subject, body subject + `\n`+paths, agent = committer else first session), pr (ref = ledger key, title `PR owner/name#n (role)`, body `PR repo#n url` + `\n<role> <source>` per entry, at = newest entry), decision. Sources pulled before each Search: `SearchDocs(since, 500)` until short page or no progress (`search.go:164-207`).

`hideMessages`: regex `(?is)<irori-message\b([^>]*)>.*?(?:</irori-message\s*>|\z)`, key attr `\bkey="([^"]*)"`; replaced by `[an Irori message, hidden: only its sender and recipients can read it]` unless viewer is "" / owner / sender (`search.go:234-264`).

ParseRef (`internal/search/search.go:64-90`): kind case-insensitive; commit id 7-40 hex lowercased; pr id `<owner/name>#<digits>`; no whitespace in id. Page (`search.go:184-204`): default 4000, max 6000 bytes, rune-snapped both ends. Limits: `search.go:48-56`.

### 2.8 Show renderings and other views (agents read this text)

Exact formats in `internal/capture/show.go:148-486` (turn, edit, commit, pr, decision, agent) and `overview.go:86-212`; stamp = UTC RFC3339 seconds or `-` (`show.go:77-82`); `block` = `"\n"+label+":\n"+trimmed body+"\n"` skipped if empty (`show.go:32-38`); Related ≤12, unique, not self. Timeline (`internal/capture/views.go:47-135`): default 20 turns, max 100, open turns of a live session first (not counted against the limit), entries sorted newest first (stable), `More` when a turn past the limit exists; subagent turns/summaries hidden unless `Subagents`. Edits default 50/max 500; Commits default 50/max 500 ordered by commit At then seenAt; Commit(sha) accepts unique ≥7-char prefix. Why (`why.go:97-199`): ≤10 edits/turns; blame first token sha. Decision id: `d-` + UTC `20060102T150405.000000` + `-` + 6 hex random (`why.go:288-292`). Overview constants: 12 agents, 5 lines/list, 120 runes; hint text `overview.go:156-162`.

### 2.8a View JSON the window and tools read (Wails today, Tauri commands in the port)

| Type | JSON | Ref |
|---|---|---|
| Entry | `{kind: turn\|summary\|commit\|pr, at, turn?, summary?, commit?, pr?}`, exactly one pointer set | `capture.go:174-192` |
| TurnView | TurnRow fields (hand-flattened, row `id` wins) + `open?` + `edits?` | `capture.go:195-199` |
| CommitView | CommitRow fields + `link?` | `capture.go:202-205` |
| Timeline | `{key, entries, more}`; `entries` always an array (`views.go:53`) | `capture.go:208-214` |
| WhyResult | `{repoKey, root, path, line?, blameSha?, commit?, edits (always array), turns?, decisions?}`; DecisionView = decision.Listed fields + `status?` | `capture.go:244-260`, `why.go:102` |
| Change | `{kind: turn\|edit\|commit\|decision\|pr\|open, key?, repoKey?}` | `capture.go:275-292` |
| Status | `{files, backlog, repos, lastScan, backfilling, lastError?, dropped{artifact,outsideRepo,ignored,overCap,unchanged}}` | `capture.go:295-305`, `noise.go:59-65` |
| Overview | `{for, repoKey?, agents[], moreAgents?, inboxOpen, inboxHeld?, inbox?, blockers?, tasks?, moreTasks?, commits?, prs?, hint}`; AgentLine `{key,name,status,repoKey?,doing}`; Line `{ref,text,at (omitzero)}` | `overview.go:31-81` |
| Hit / Shown | `{ref,kind,at,title,snippet,agent?,repoKey?}` / `{ref,kind,title,text,more,next?,related?}` | `internal/search/search.go:126-161` |

`TimelineQuery`, `EditQuery`, `WhyQuery`, `DecisionInput`, `SearchQuery`, `ShowOptions` have no JSON tags in Go (`capture.go:217-273`); the Tauri command signatures are the port's to define (match whatever the frontend's current Wails binding calls pass).

### 2.9 attach protocol (`internal/attach/protocol.go`)

- Version 2; `AppendSince = "0.4.0"`; event types hello, deliver, append, name, held, moved, inbox, superseded, error (`protocol.go:14-64`). Event JSON fields (all omitempty): `type, v, session, agentKey, name, nameSource, aiTitle, api{url,token}, attachSecret, wakeId, token, reminder, deliveryIds, messageIds, prompt, from, mode, overflow, deliveryId, why, preview, to, unread, held, message` (`protocol.go:67-118`). Headers `X-Irori-Attach`, `X-Irori-Member`, `X-Irori-Origin` (`protocol.go:121-125`).
- `ModAtLeast(v,min)` (`internal/attach/version.go:9-52`): trim spaces, strip leading `v`, SplitN 3 on `.`, each part leading digits (max 6), a suffix ends the version, missing parts 0; unreadable v → false; unreadable min → true. Vectors: `version_test.go:6-36`.
- Runner (`internal/attach/runner/runner.go`): read lockfile `<DataDir>/irori.json` (`{pid,port,token,version,deviceId,headless,startedAt}`, `internal/discovery/discovery.go:17-28`), pid must be alive; probe `GET /v1/health` with 2 s timeout (`internal/api/client.go:23,69-90`), require ok and matching deviceId; then `GET /v1/attach?session=<id>[&mod=<ver>]` with `Authorization: Bearer <token>`, `Accept: application/x-ndjson`, no timeout (`internal/api/client_v02.go:97-120`). Non-200 → StatusError (404 = "not on the roster yet").
- Copy: lines read with a 64 KiB reader, max 1 MiB per line (oversized dropped whole), trimmed; empty = keepalive (server sends bare `\n` every 15 s, `internal/api/routes_events.go:25`); non-JSON skipped; each JSON line written to stdout + `\n` **unchanged**; `hello` → connected; `superseded` → exit 0 (`runner.go:124-196`).
- Reconnect: backoff 1 s doubling to 16 s, reset after a hello; first non-HTTP failure of an outage emits one stdout line `{"type":"error","message":"Irori isn't running, so messages can't reach this session. It attaches by itself once Irori starts."}`; stdout write failure → exit with error (`runner.go:26-120`).
- Process (`attach.go:25-112`): flags `--session` (required), `--cwd`, `--mod-version` (Go `flag` also accepts `-session`, `--session=x`); stdin ignored (the mod's spawn closes it); SIGINT/Ctrl-C ends; parent watch: Windows wait on parent handle, macOS kqueue NOTE_EXIT, else poll every 5 s (`Alive`, reparented `ppid<=1 || getppid()!=ppid` off Windows, start time after ours = reuse). Windows release binary is `-H windowsgui` (no console); logs to stderr with `log.LstdFlags`.

---

## 3. Rules, constants, timings

| Constant | Value | Ref |
|---|---|---|
| ScanInterval / RepoInterval / SafetyInterval | 3 s / 5 s / 1 min | `capture.go:45-47` |
| QuietAfter | 2 min | `capture.go:48` |
| BackfillWindow / BackfillMaxBytes | 72 h / 64 MiB | `capture.go:49-50` |
| poolAge / cmdAge / relinkAge / commitSlack / gitTimeout | 30 d / 24 h / 1 h / 2 s / 15 s | `service.go:26-41` |
| saveEvery / readBuf / maxLine | 4 MiB / 1 MiB / 64 MiB | `tailer.go:21-28` |
| busyRecheck / eventBatch / events chan | 15 s / 512 / 64 | `watch.go:21-24,66` |
| maxReflogRead / repoBudget / maxReflogTries | 4 MiB / 1500 ms / 3 | `repos.go:22-35` |
| negativeTTL (root cache "not a repo") | 1 min; timeouts not cached; positives forever | `roots.go:16,59-79` |
| maxShellEdits / ignoreCacheMax / ignoreArgBytes / noiseTurnsMax | 50 / 50000 / 8000 / 4096 | `internal/capture/noise.go:23-35` |
| maxCommand | 300 runes | `rows.go:20` |
| MaxWalk / MaxFiles / CommitterWindow / maxSubject | 200 / 500 / 10 s / 200 | `gitlink.go:14-21`, `describe.go:13` |
| repo gitTimeout / cacheTTL | 2 s / 5 min (also caches "" ) | `resolver.go:15-18` |
| pullPage | 500 | `search.go:81` |
| fswatch debounce default / chan sizes / RDCW buffer | 50 ms / 1024 / 64 KiB | `internal/fswatch/fswatch.go:51-55`, `fswatch_windows.go:14,48-49` |
| FSEvents latency | 0.01 s | `fswatch_darwin.go:38` |
| attach backoff / maxLine | 1-16 s / 1 MiB | `runner.go:27-34` |
| parentPollEvery | 5 s | `attach.go:83` |

Behaviour rules:
- **Backfill / start offset** for a file not in the cursor file (`tailer.go:267-280`): size ≤ 64 MiB and mtime within 72 h → 0; else if live and size > 64 MiB → `boundaryStart` (offset of the last line containing `"subtype":"compact_boundary"` in the last 64 MiB, else the first whole line in that window; a window that starts mid-line skips its first fragment, `tailer.go:285-324`); else → end of file. A cursor beyond the size → 0. File shrank → restart at 0 with a fresh parser (`tailer.go:379-383`).
- **Cursor = open turn's first line** while a turn is open, else bytes read; rows re-read after restart dedupe by id (`tailer.go:49-57`, `readFile` `tailer.go:425-486`). Only complete lines (ending `\n`) are consumed. Cursor saved every 4 MiB and at end of each pass (temp `capture-cursors.json.*.tmp`, fsync, rename; only when dirty) (`cursors.go:61-95`). A damaged cursor file = empty (re-read). Failed append → reset parser to last saved cursor.
- **Tail order**: live sessions first, then newest mtime (`tailer.go:179-187`).
- **Quiet close**: open turn and `now - lastGrow ≥ QuietAfter` and not (live and card busy) → Close(lastActivity or now, "quiet") (`tailer.go:395-404`); in event mode a timer arms for the earliest due; a busy one is rechecked after 15 s (`watch.go:446-498`).
- **Backfilling** flag clears after the first full pass (`tailer.go:237-240`).
- **Repo first sight** (`rows.go:92-132`, `repos.go:81-127`): noteRoot logs repo.seen and snapshots logs/HEAD size. Reflog start offset = current size, or the snapshot if smaller, or the first line at/after the root's oldest captured edit time (floored to now-72 h, truncated to seconds, searching only the last 4 MiB) if earlier. Size < offset (reflog expire) → jump to end. Per line: import each entry; budget checked after every imported commit and every line; a failing line is retried on the next pass up to 3 tries then skipped.
- **Op** = reflog message up to first `:`, trimmed (`repos.go:228,249`).
- **Linking** (`gitlink.go:119-133`, `internal/capture/gitlink/classify.go:20-68`, candidates `repos.go:269-323`): candidates = repo's uncommitted pool edits with `At ≥ now-30d` and `At ≤ commit.At+2s`, plus already linked non-heuristic edits whose AfterBlob/AfterBlobLF equals a committed blob for the same path (and their link-mates), sorted by At then ID. Per file: exact (newest blob match, non-heuristic, any root, ties → later candidate; plus earlier edits same Root+Path), else partial (same root, path), heuristic if all heuristic, else unattributed. Deleted files (status D, empty/zero blob) never blob-match. Empty commit root matches no edit.
- **Committer** (`classify.go:86-123`): cmd Cwd under root (normPath), cmd span [Start,At] within ±10 s of commit At, distance 0 inside span, closest then latest At. Commands pre-filtered with `spans` = ±20 s (`cmddir.go:183-190`).
- **Relink** after each tail: commits seen < 1 h ago whose non-exact files now have a fresh blob-matching edit get a new commit.linked (`repos.go:398-477`); late commit commands give committer-less links a committer (`repos.go:543-597`); `linkUnlinked` after every repo pass (`repos.go:378-393`).
- **IsCommitCommand** (`internal/capture/gitlink/command.go`): lexer for quotes, backslashes, `#` comments, separators `; & | ( ) \n` backtick and `$(`; skips assignments and wrappers (sudo, env, …, shell keywords) with value flags; `git [global opts] commit|merge|cherry-pick|revert|rebase|am|pull` unless `--abort/--quit/-h/--help` (value options skipped); `gh pr merge`; `bash|sh|zsh|dash -…c <cmd>` and `eval` reparsed (depth ≤4).
- **commandDir** (`cmddir.go:27-100`): segments on `&& || ; | \n` outside quotes (single `&` not a separator); follows `cd/set-location/sl/pushd/push-location/chdir [-Path|-LiteralPath] <dir>` (`-` ignored), first committing `git` segment applies its `-C`s; expands `~ $HOME ${HOME} $env:USERPROFILE $env:HOME %USERPROFILE%` prefixes, Git Bash `/c/…` → `C:/…`; relative joined to current.
- **Noise filter for heuristic edits** (`noise.go:15-22,192-273`): drop artifacts (extension list `noise.go:41-49`, any segment `.git`/`node_modules`), +0-0, outside every repo, git-ignored (batched check-ignore ≤8000 arg bytes, failure keeps), >50 per tool call. Unsure owner (rev-parse failed under a `.git`, or inside a session cwd that no longer exists) → keep. Dropped paths are removed from the turn's EditPaths unless another kept edit names them. Counts in Status.Dropped (not persisted).
- **PR detection** only when gitOperation present or the command contains `gh` (`rows.go:274-276`).
- **Hot roots**: only roots a live session used keep an open logs-dir watch (open handles block renaming/removing the repo folder); others are read by the safety pass (`watch.go:348-401`).
- **Event loop** (`watch.go:63-157`): watch projects (recursive) before the first pass; batch ≤512 events; Overflow or closed projects watcher → full tail pass; `.jsonl` paths mapped to listFiles spelling (`watch.go:252-268`) → tail those; logs `HEAD` event → read that root; closed logs watcher → poll at RepoInterval; reflog follow-up immediate if budget ran out or progress was made, else after RepoInterval.

repo keys (`resolver.go:55-132`): `filepath.Abs(dir)`; cache byDir and byRoot 5 min; `rev-parse --show-toplevel` → `remote get-url origin` normalized → `remote:`; else smallest line of `rev-list --max-parents=0 HEAD` → `root:`; timeouts never cached. NormalizeRemote rules `internal/repo/normalize.go:16-152` (schemes http, https, ssh, git, git+ssh, ssh+git, git+https; scp-like needs `:` before any `/` and host > 1 char; per-segment percent-decode refusing `/` or `\`; lowercase; drop empty segments, trailing `.git`; refuse `.`/`..`/control chars; Azure folds; ≥2 segments).

scrub (`scrubber.go`): exact values ≥8 bytes, deduped, longest first, single pass replace with `[redacted:exact]`; then PEM private-key blocks (BEGIN…END or to end of text, each later line keeps only an optional `+`/`-`/space diff marker and indentation); then per line the rules in order jwt, anthropic, openai (needs a digit after `sk-`), github, slack, aws, google, bearer (credential = ≥8 chars and non-letter or ≥16), authorization basic/token/digest/negotiate (≥8), URL userinfo password, assignment (`scrubber.go:163-305`), each gated by a literal or case-folded word. Values ≥8 chars, not already a marker, not "code" (`scrubber.go:320-359`). SkipFile names: `skipfile.go:10-42`. Strict paths → `[path]`: `strict.go:37-50`.

attach-path rules (`internal/blob/attachpath.go`): refuse SkipFile names, any element in `.ssh .gnupg .aws .azure .kube .docker .password-store keychains` (not last), final name in `.git-credentials .credentials.json .pgpass irori.json device.key person.json`, suffix `.keychain .keychain-db .kdbx .ppk .gpg`, `microsoft\credentials|protect`; checked on the given and the symlink-resolved path; inside DataDir refused except `received/`; must be a regular file. `within` is case-insensitive on Windows and macOS. SafeName rules `attachpath.go:153-214` (120 runes keeping ext ≤16 runes, reserved device names incl. `CONIN$`, `CONOUT$`, superscript digits).

---

## 4. Platform-specific code

| Concern | Windows | macOS | Other |
|---|---|---|---|
| Process alive | `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`; ACCESS_DENIED = alive; `GetExitCodeProcess == 259` (`internal/proc/proc_windows.go:11-23`) | `kill(pid,0)` (EPERM = alive) and sysctl `kern.proc.pid` `p_stat==SZOMB(5)` = dead (`proc_darwin.go:20-29`) | `kill(pid,0)` |
| Start time | `GetProcessTimes` creation FILETIME (`proc_windows.go:25-36`); `FiletimeTicks` = 100 ns since 1601, 18 digits, compared to Claude's `procStart` within 1 s (`internal/presence/live_windows.go:7`) | `kinfo_proc.kp_proc.p_starttime` (sec,usec) (`proc_darwin.go:33-43`); presence compares Unix-second procStart ±2 s | none |
| WaitExit | `OpenProcess(SYNCHRONIZE\|QUERY_LIMITED)`; ERROR_INVALID_PARAMETER = gone; reuse check by creation time; `WaitForMultipleObjects([proc, stopEvent])` (`wait_windows.go`) | kqueue `EVFILT_PROC NOTE_EXIT` + `EVFILT_USER` wake; ESRCH = gone; reuse check after registering (`wait_darwin.go`) | ErrCantWait → poll |
| fswatch | one `ReadDirectoryChangesW` overlapped handle, 64 KiB buffer, mask FILE_NAME\|DIR_NAME\|SIZE\|LAST_WRITE, share R/W/D, armed before Watch returns; `ERROR_NOTIFY_ENUM_DIR` or n==0 → Overflow; other error → Overflow + stop (`fswatch_windows.go`) | FSEvents stream, `FileEvents\|NoDefer\|WatchRoot`, since-now, on an owned thread's CFRunLoop; MustScanSubDirs/UserDropped/KernelDropped/RootChanged → Overflow; HistoryDone ignored; realpath mapped back to caller root (case-insensitive fallback); non-recursive filters depth (`fswatch_darwin.go`, `.c`) | ErrUnsupported → polling loop (`tailer.go:59-85`) |
| Mark of the Web | write `<path>:Zone.Identifier` = `[ZoneTransfer]\r\nZoneId=3\r\n` (`motw_windows.go:9-11`) | xattr `com.apple.quarantine` = `0081;%08x(unix now);Irori;` (`motw_darwin.go:16-19`) | no-op |
| git no console | `CREATE_NO_WINDOW` + HideWindow | – | – |
| env name compare | case-insensitive | case-sensitive | case-sensitive |
| normRoot | lowercase | as-is | as-is |
| blob openRetry | sharing violation (32) / access denied (5) retried | – | – |
| attach parent | never reparents | reparented check | poll |

---

## 5. Rust crates (versions from crates.io, 2026-10-29)

| Need | Crate | Version | Notes |
|---|---|---|---|
| JSON | `serde` + `serde_json` | 1.x / 1.0.154 | custom time (de)serializer; lossy UTF-8 pre-pass (section 7) |
| time | `chrono` | 0.4.45 | `DateTime<FixedOffset>`; custom Go RFC3339Nano formatter |
| sha256 / sha1 | `sha2` / `sha1` | 0.11.1 / 0.11.0 | ids, blobs, git blob ids |
| hex | `hex` | 0.4.3 | |
| gzip | `flate2` | 1.1.10 | `MultiGzDecoder` to read Go's gzip |
| regex | `regex` | 1.13.1 | ASCII `\b`/`\s` (section 7) |
| exact-value replace | `aho-corasick` | 1.1.5 | patterns longest first, `MatchKind::LeftmostFirst` |
| SQLite FTS5 | `rusqlite` (`bundled`) | 0.41.1 (published 2026-10-28; 0.41.x) | bundled build enables FTS5; porter is built into FTS5. Keep a port of `TestFTS5Available` |
| fs watch | `notify` | 8.2.1 | RDCW / FSEvents backends; verify overflow → Rescan maps to Overflow, else hand-roll with `windows`/`fsevent-sys` 5.2.0 |
| Windows APIs | `windows-sys` | 0.61.2 | OpenProcess, GetProcessTimes, WaitForMultipleObjects, RDCW |
| macOS | `libc` | 0.2.191 | sysctl kinfo_proc, kqueue; or `nix` 0.31.3 (`event` feature) |
| xattr | `xattr` | 1.7.0 | quarantine |
| URL parse | `url` + `percent-encoding` | 2.5.8 / 2.3.2 | see trap on Go url.Parse |
| paths | `dunce` | 1.0.5 | canonicalize without `\\?\`; `path-clean` 1.0.1 is not Go's Clean on Windows: port `filepath.Clean/Rel/Join/Abs` by hand |
| temp + atomic rename | `tempfile` | 3.27.0 | `persist` = rename; keep the "never replace existing blob" check |
| mime | `mime_guess` 2.0.5, `infer` 0.22.0 | | not identical to Go's tables (section 7) |
| attach HTTP | `ureq` | 3.5.0 | blocking, small; fits a CLI child. reqwest 0.13.5 + tokio 1.54.1 if the app already runs tokio |
| async runtime (app) | `tokio` | 1.54.1 | capture loop, timers |

---

## 6. Tests to port (what each proves)

Fixtures: `transcript/fixture_test.go` builds synthetic rows (`prompt`, `assistant`, `textBlock`, `thinking`, `toolUse`, `result`, `sys`, `turnDuration`, `feed`) from a fixed base time with `ts(n)` RFC3339 strings; `harness_test.go` has a fake roster (busy/idle/gone cards), a manual clock, real temp git repos (`newRepo(t, autocrlf)`), and a rig with `tail()`, `repos()`, `restart()`, `rows(kind)`, `counts()`, `session(sid,cwd)` transcript writer.

**transcript** (`internal/capture/transcript/*_test.go`): TurnBoundaries (next/turn_duration/close reasons), TurnDurationWithoutOpenTurn (no-op), CloseQuiet, Interrupt (both texts), Origin (object/string/sdk/command/peer/agent), PromptText (string vs blocks join), MetaIsNotPrompt, Summaries (all five kinds, Full when clipped), PRLink (string/number/#), IgnoredLines (api-request, bad JSON, other agent), CRLFLine, Sidechain, ImplicitTurn (unknown origin, uuid id), Edits (Edit/Write/NotebookEdit/before/after/replaceAll/create hunk), Bash (stdout clip, gitOperation, string error result), EditPathsDedupe, TurnEmittedAfterItsItems, LateResult (result after turn closed: edit emitted, Done not set), ToolsCap (200 + ToolsMore), OpenIsACopy, LastActivityNewest, FullSession (each Item has one field), SamePromptID / SamePromptIDLater (continuation vs uuid fallback), ResultAfterTurnClosed, PowerShell / PowerShellTarget, SourceFor / SkipLine, Target (relpath, clip), UnifiedDiff, Clip (whitespace backoff).

**gitlink**: ParseReflog / ParseReflogZone / ZeroSHA / GitDir (dir, file, relative, BOM) / ReflogMovements (real repo: commit, ff from removed worktree, checkouts) / ReflogPartialLineOnDisk / NewCommitsMaxWalk / NewCommitsFake; Describe / DescribeMaxFiles / ParseRawZ / DescribeFakeErrors; Classify (all four classes, run of earlier edits) / ClassifyTwoRoots / ClassifyRealBlobs (gitsha vs real git, with and without autocrlf) / ClassifyFastForwardFromRemovedWorktree / Committer (window, span, tie → latest) / UnderRoot (slashes, case, drive roots, `/c/`); IsCommitCommand / SplitCommands (wrappers, quotes, `bash -c`, eval, heredoc false positive accepted) / Exec / GitEnv (repo vars stripped).

**capture service** (`capture_test.go`, `review_test.go`, `noise_test.go`, `watch_test.go`, `search_test.go`, `cmddir_test.go`, `fts_test.go`): EndToEnd (edit→commit→exact link→timeline), CRLFLinksExact (AfterBlobLF), PowerShellCommitter, FastForwardFromRemovedWorktree, TwoWorktreesSamePath, RestartNoDuplicates, CrashBeforeCursorReplaysWithoutDuplicates, BackfillRules (the three start-offset cases), SubagentAttribution, PRLedger, Decisions, ScrubbedSecretsNeverStored (rows and blobs), QuietClose, TimelinePaging, RelinkLateEdits, RunAndSubscribe, CommitBeforeRootFirstSeen (oldest-edit reflog rewind), RepoPassBudget, LateCommitCommandSetsCommitter, LateCommitCommandsLinkOnce; AmendKeepsExactLink, LinkedEditsNeverMakePartial, CommitBeforeFirstRepoPass, TransientGitFailureRetries, StraddlingSecretNotStored (clip backoff + scrub), CommitSubjectScrubbed, StuckReflogLineSkipped (3 tries); HeuristicNoiseDropped, HeuristicCapPerCall, CheckIgnoreFailsOpen, IsArtifact, HeuristicDeletedWorktreeKept, HeuristicRevParseFailsOpen, HeuristicUnchangedDropped; EventTailNewAndAppended, EventQuietCloseTimer, EventQuietBusyRecheck, EventReflog, EventWorktreeRemovable (Windows handle must not block `git worktree remove`), EventReflogFollowUp, EventBusyLoopLosesNothing, EventGoneRootReleased; MergeOwner, MatchExpr, SearchCaptureKinds, SearchRankingAndFilters, SearchLimits, SearchSnippet, SearchPullsSources, SearchPullPaging, AddSourcePanics, Show, ShowPaging, ShowRelatedCap, ShowOpenTurn, Overview, OverviewCapsAndRepoFallback, SearchOffsetPaging, SearchViewer, HideMessages; CommandDir; FTS5Available.

**scrub**: TokenShapes, PrivateKey, Bearer, Assignment, EnvFile, FalsePositives (identifiers, type annotations, prose), Exact, Concurrent, LineCount, BigPatchTime / LongLineTime (performance bounds; `race_on_test.go`/`race_off_test.go` set `slow = 10` under `-race`, else 1, as a multiplier on the time bounds), ContainsAnyFold, ReviewFindings / ReviewFalsePositives (`regress_test.go`, regression vectors: port verbatim), ZeroScrubber, SkipFile, SkipFileWindowsNames, StrictDropsPaths.

**blob**: store tests (round trip, gzip on disk, idempotent, truncation and its id, bad id, not found, directory is not a blob, corrupt blob, concurrent same content / put+get / two stores / stress, put while final held open (Windows), dir-is-file failure, never replace) and files tests (PutResumeAndFinish, Cap, CheckAttachPath, SafeName), MarkDownloaded (Windows ADS, macOS xattr).

**fswatch**: create/append/nested, close and ctx stop, non-recursive, no leaked threads; Windows: lost events always yield Overflow, stop-after-self-close signals nothing, slow reader gets Overflow.

**repo**: NormalizeRemote table (`internal/repo/normalize_test.go:6-80`; note line 79 is `"git@github.com:o/n\x01"` with a literal 0x01, which is why it maps to ""), SmallestSHA, cache per dir and toplevel, root-commit fallback, negative cache, timeout not cached, concurrency, integration (root commit, remote, local remote falls back, clones match, not a repo, empty repo, inherited GIT_DIR ignored, relative dir follows cwd).

**proc**: SelfIsAlive, SelfStartTime, FiletimeMatchesClaudeUnits (18 digits), ChildStartTime / NoStartTimeForMissingPid / ZombieIsNotAlive (macOS), WaitExit (exits, killed, canceled, reused pid).

**gitsha**: known vectors (empty `e69de29b…5391`, `hello\n` `ce013625…464a`), Pair. **attach**: ModAtLeast table. **search**: ParseRef, Page.

---

## 7. Hard parts and compatibility traps

1. **Go path semantics.** `filepath.Clean` (lexical `..`, `/`→`\` on Windows, drive/UNC handling), `filepath.Rel` (case-insensitive on Windows, used by `relTo` `roots.go:102-111` and `within` `attachpath.go:133-142`), `Join`, `IsAbs`, `Abs` (GetFullPathName), `EvalSymlinks` (no `\\?\`). `Path::strip_prefix` is case-sensitive and `canonicalize` adds `\\?\`. Port these functions by hand as a `gopath` module with Go's test vectors; normRoot and the cursor file depend on them.
2. **Lowercasing.** Go `strings.ToLower`/`EqualFold` are per-rune simple mappings; Rust `to_lowercase()` is full mapping (`İ` → `i̇`, length changes). normRoot output is persisted (cursor `reflogs` keys), pathKey/editPathKey/ignoreKey compare. Use a per-char simple lowercase (first char of `to_lowercase()` for 1:1 mappings, Go-table for the rest) and a simple-fold EqualFold.
3. **Regex flavour.** Go RE2 `\b`, `\s`, `\w` are ASCII; Rust `regex` is Unicode by default. In scrub (`scrubber.go:163-318`), strict (`strict.go:37-49`), hideMessages: write `(?-u:\b)`, and spell `\s` as `[\t\n\f\r ]` inside classes. Go `(?i)` matches Unicode simple folds; Rust too. Rule order and the gate checks must stay identical, and the result must keep line counts.
4. **Exact replacer.** `strings.NewReplacer` scans left to right and tries patterns in argument order (sorted longest first) → aho-corasick LeftmostFirst with the same order.
5. **JSON decoding differences.** Go: case-insensitive keys, last duplicate wins, type mismatch drops the whole line, invalid UTF-8 / lone surrogates → U+FFFD. serde: case-sensitive, duplicate is an error by default for structs, invalid UTF-8 errors. Do: `String::from_utf8_lossy`, then parse to `serde_json::Value` (or a raw map), look up keys case-insensitively, apply "drop the line on a known-field type mismatch" for `row`, and field-by-field tolerance for toolUseResult. `contentBlocks` per-element fallback. Go also keeps the partially decoded value on a mismatch inside an array (`edits`); rare.
6. **JSON encoding.** Embedded structs flatten and the shallower `id` wins (TurnRow hides Turn.id). `#[serde(flatten)]` would emit both: write TurnRow (de)serialization by hand. Zero time writes `0001-01-01T00:00:00Z` despite omitempty; nil slices without omitempty write `null`. Go's time format trims fractional zeros (chrono's `AutoSi` doesn't: custom formatter). Keep each value's offset (`Z` vs `-06:00`) by using `DateTime<FixedOffset>` and writing `now()` in local offset. HTML escaping parity: see `docs/port/store.md` (signed rows).
7. **Hash inputs are raw strings.** Edit id hashes the transcript's path string (I1b); summary id hashes scrubbed text, so a scrub rule change makes old title summaries (no rowUuid) re-emit once. Do not "improve" scrub rules during the port.
8. **Clip functions differ**: `transcript.clip` backs up to whitespace (`target.go:81-99`), `clipRunes` doesn't (`rows.go:328-334`), `truncRunes` adds no ellipsis (`describe.go:96-106`), `clipBytes` byte-cap on rune start (`target.go:102-111`). Go `unicode.IsSpace` ≈ Rust `char::is_whitespace`; Go `unicode.IsLetter/IsDigit` are categories L and Nd (Rust `is_alphabetic`/`is_numeric` are wider), which matters in `matchExpr` (`index.go:196-213`) and SafeName (`Cf`, `IsControl`).
9. **Go `url.Parse` vs `url` crate** in NormalizeRemote: Go keeps the raw path for EscapedPath, accepts odd hosts, `Hostname()` strips port and IPv6 brackets. The `url` crate normalizes special-scheme paths (`/o/../n` resolved, `%2E%2E` handled) before you can refuse them: check the raw string for `.`/`..` segments and `%2F`/`%5C` before parsing, then run the whole test table.
10. **Time zones in commit rows.** `commit.seen.at` is local wall time of the importing device; compare as instants only. Reflog times carry the line's tz.
11. **git binary lookup.** Go `exec.LookPath` won't run `git.exe` from the current dir; Rust's Windows `Command` search also checks the exe's own directory first. Resolve `git` from PATH (with PATHEXT) yourself. Kill-on-timeout plus a 500 ms WaitDelay for pipes; CREATE_NO_WINDOW on every spawn.
12. **Watch handles block folder renames on Windows.** Only hot roots (live session used) may hold a logs-dir watch (`watch.go:348-357`), tested by EventWorktreeRemovable / EventGoneRootReleased. notify's RDCW backend also holds a directory handle; keep the same hot-set rule. Overflow must never be lost (sticky Overflow in `coalesce.go:18-73`), and the first read must be armed before Watch returns.
13. **Single-goroutine assumptions.** Tailing, repo passes, quiet timers and follow-ups share one loop; the repo budget (1.5 s) exists so tailing isn't starved. Keep one task owning tfile/reflog state; readers use the RwLock'd fold. Duplicate `commit.linked` rows were once seen live and their cause is still unexplained (`repos.go:536-542`); `relinkMu` plus a re-check before emit is defence only.
14. **Replay ordering.** Replay pages by store cursor `(hlc,device,seq)`, oldest first; later commit.linked replaces earlier; linked edits leave the pool; commit cmds older than 24 h are dropped after replay (`service.go:203-210`).
15. **Blob store races.** Two processes (Go and Rust side by side during testing) may write the same blob: never rename over an existing final (Windows open handles make replace fail); treat "final exists" as success; retry opens on sharing violations.
16. **SHA-256 repos.** isSHA accepts 64-hex ids but gitsha only computes SHA-1, so exact links never fire there (same in Go; keep).
17. **`commandDir` output is not cleaned** (mixed slashes stored); `underRoot` normalizes later. Reproduce, don't clean.
18. **FTS5 behaviour** depends on the SQLite build (modernc vs bundled libsqlite3): bm25 and snippet are stable, but run the search tests against the bundled build.
19. **Mime sniffing.** Go `mime.TypeByExtension` reads the OS registry on Windows and `/etc/mime.types` elsewhere; `http.DetectContentType` is a fixed WHATWG table. Only FileRef.Mime (display) depends on it; port the sniff table if exact parity is wanted.
20. **macOS kinfo_proc layout** for start time and zombie state needs a hand-laid struct (or libc's `kinfo_proc`); FSEvents must run on its own CFRunLoop thread; none of this was run on a Mac during this read.
