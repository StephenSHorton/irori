//! Merge desk slice 2 (docs/design/merge-desk.md §3, §5 "Trust"): which
//! card is a PR's author card, and whether the PR is ours: by the person,
//! a member of their spaces, or one of their agents (card #625). The desk
//! shows only ours; anyone else's PR is not listed at all.
//!
//! Pure: it reads the cards and the ledger's author it is given and writes
//! nothing. The desk poller writes the `board_link` and makes the "PR #n"
//! card from what it returns.

use coord::Task;

/// What the mapping needs from one row of the desk's `gh pr list` snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrRef {
    /// `owner/name`, any case.
    pub repo: String,
    pub number: i64,
    pub head_ref: String,
    /// The GitHub login that opened it.
    pub author_login: String,
    /// Untrusted text: never shown unquoted.
    pub title: String,
}

impl PrRef {
    /// A row of the desk's snapshot, for the repo `owner/name`.
    pub fn of(repo: &str, p: &coord::DeskPr) -> PrRef {
        PrRef {
            repo: repo.into(),
            number: p.number,
            head_ref: p.head_ref_name.clone(),
            author_login: p.author.clone(),
            title: p.title.clone(),
        }
    }
}

/// Which §3 rule found the card. Order is the rules' order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    /// 1: a card already links the PR.
    Linked,
    /// 2: the ledger's author session holds it (branch match, else its only
    /// held card).
    Ledger,
    /// 3: an open card whose branch is the PR's head branch.
    Branch,
    /// 4: none; the desk makes a "PR #n" card.
    None,
}

/// A PR's author card, and the agent behind it when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub rule: Rule,
    /// The card, empty for [`Rule::None`].
    pub task_id: String,
    /// The ledger's author session, if the ledger has one.
    pub author: String,
}

/// The desk's view of whose PRs are ours (§5 "Trust", card #625).
pub struct Trust<'a> {
    /// Whether an agent key is the person's agent or a space member's.
    pub own_agent: &'a dyn Fn(&str) -> bool,
    /// The GitHub logins whose PRs are ours: the person's, their members'
    /// and the desk's `trusted` list ([`desk_logins`]).
    pub logins: &'a [String],
}

fn open(t: &Task) -> bool {
    !coord::is_terminal(&t.state) && !t.withdrawn
}

fn same_repo(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The repo a `remote:github.com/owner/name` key names, or "".
fn key_repo(k: &str) -> &str {
    k.strip_prefix("remote:github.com/").unwrap_or("")
}

/// A card that could be about this PR's repo: one with no repo, or this one.
fn repo_ok(t: &Task, pr: &PrRef) -> bool {
    t.repo_key.is_empty() || same_repo(key_repo(&t.repo_key), &pr.repo)
}

fn branch_of(t: &Task) -> &str {
    t.where_.as_ref().map(|w| w.branch.as_str()).unwrap_or("")
}

fn links_pr(t: &Task, pr: &PrRef) -> bool {
    t.links
        .iter()
        .any(|l| coord::pr_link(l).is_some_and(|(r, n)| n == pr.number && same_repo(&r, &pr.repo)))
}

/// The newest of `ts` (card ids are ULIDs, so the largest is the newest).
fn newest<'a>(ts: impl Iterator<Item = &'a Task>) -> Option<&'a Task> {
    ts.max_by(|a, b| a.id.cmp(&b.id))
}

/// Maps `pr` to its author card by the §3 rules, first match wins.
/// `author` is the ledger's author session for the PR, or "".
pub fn map_pr(pr: &PrRef, tasks: &[Task], author: &str) -> Mapping {
    let found = |rule, t: Option<&Task>| {
        t.map(|t| Mapping {
            rule,
            task_id: t.id.clone(),
            author: author.into(),
        })
    };
    // 1. A card that links the PR: an open one first.
    let linked = || tasks.iter().filter(|t| links_pr(t, pr));
    if let Some(m) = found(Rule::Linked, newest(linked().filter(|t| open(t))))
        .or_else(|| found(Rule::Linked, newest(linked())))
    {
        return m;
    }
    // 2. The author session's held card on the PR's branch, else its only
    //    held card.
    if !author.is_empty() {
        let held: Vec<&Task> = tasks
            .iter()
            .filter(|t| open(t) && t.holders.iter().any(|h| h == author) && repo_ok(t, pr))
            .collect();
        let on_branch = newest(
            held.iter()
                .copied()
                .filter(|t| !pr.head_ref.is_empty() && branch_of(t) == pr.head_ref),
        );
        let only = (held.len() == 1).then(|| held[0]);
        if let Some(m) = found(Rule::Ledger, on_branch.or(only)) {
            return m;
        }
    }
    // 3. Any open card on the PR's branch.
    if !pr.head_ref.is_empty()
        && let Some(m) = found(
            Rule::Branch,
            newest(
                tasks
                    .iter()
                    .filter(|t| open(t) && repo_ok(t, pr) && branch_of(t) == pr.head_ref),
            ),
        )
    {
        return m;
    }
    Mapping {
        rule: Rule::None,
        task_id: String::new(),
        author: author.into(),
    }
}

/// Whether `pr` is ours (§5 "Trust", card #625): its ledger author is one
/// of the person's agents, a card linking it is held by theirs or a
/// member's, or its login is the person's, a member's or on the desk's
/// list. A branch match (rule 3) never makes a PR ours: anyone can name a
/// branch.
pub fn trusted(pr: &PrRef, m: &Mapping, tasks: &[Task], trust: &Trust) -> bool {
    if !m.author.is_empty() && (trust.own_agent)(&m.author) {
        return true;
    }
    if m.rule == Rule::Linked
        && let Some(t) = tasks.iter().find(|t| t.id == m.task_id)
        && t.holders
            .iter()
            .chain(&t.co_holders)
            .any(|h| (trust.own_agent)(h))
    {
        return true;
    }
    !pr.author_login.is_empty()
        && trust
            .logins
            .iter()
            .any(|l| l.eq_ignore_ascii_case(&pr.author_login))
}

/// The desk's verdict on one PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Ours, with an author card: link it if it isn't linked yet.
    Card(Mapping),
    /// Ours, no card found: make a "PR #n" card (assigned to `author`
    /// when there is one) and link it.
    NewCard(Mapping),
    /// Not ours (card #625): not shown, no card, no wake, never checked.
    NotOurs,
}

/// Maps and checks `pr` in one go.
pub fn judge(pr: &PrRef, tasks: &[Task], author: &str, trust: &Trust) -> Verdict {
    let m = map_pr(pr, tasks, author);
    if !trusted(pr, &m, tasks, trust) {
        return Verdict::NotOurs;
    }
    if m.rule == Rule::None {
        Verdict::NewCard(m)
    } else {
        Verdict::Card(m)
    }
}

/// The title of a "PR #n" card: the PR's title quoted, as untrusted text.
pub fn new_card_title(pr: &PrRef) -> String {
    let t: String = pr
        .title
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    if t.trim().is_empty() {
        format!("PR #{}", pr.number)
    } else {
        format!("PR #{} {:?}", pr.number, t.trim())
    }
}

/// A PR an agent opened, from the ledger's author row (`gh pr create`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Opened {
    pub session: String,
    /// `owner/name`.
    pub repo: String,
    pub url: String,
    /// When the ledger saw it, unix ms.
    pub at_ms: i64,
}

/// How recent an opened PR must be to link it now: older author rows are
/// history replayed at start, and their session may hold other work today.
pub const OPENED_FRESH_MS: i64 = 15 * 60 * 1000;

/// The card an agent's freshly opened PR belongs on (§3, "a small change
/// upstream"): the one open card that session holds in the PR's repo,
/// unless some card already links the PR. With several held cards it
/// guesses nothing; the desk's rules 2 and 3 map it later by branch.
pub fn opened_card(o: &Opened, tasks: &[Task], now_ms: i64) -> Option<String> {
    if o.url.is_empty() || o.session.is_empty() || now_ms - o.at_ms > OPENED_FRESH_MS {
        return None;
    }
    let (repo, number) = coord::pr_link(&coord::Link {
        kind: coord::LINK_URL.into(),
        reference: o.url.clone(),
        label: String::new(),
    })?;
    let pr = PrRef {
        repo,
        number,
        ..PrRef::default()
    };
    if tasks.iter().any(|t| links_pr(t, &pr)) {
        return None;
    }
    let mut held = tasks
        .iter()
        .filter(|t| open(t) && t.holders.contains(&o.session) && repo_ok(t, &pr));
    let one = held.next()?;
    held.next().is_none().then(|| one.id.clone())
}

/// The logins whose PRs are ours on `desk`: its `trusted` list, the
/// person's own login as the poll's gh heard it (card #521), and `people`:
/// the person's and their space members' logins (card #625). An old
/// `desk.ok` row no longer adds one: there are no Outside PRs to OK.
pub fn desk_logins(desk: &coord::Desk, people: &[String]) -> Vec<String> {
    let mut logins = desk.settings.trusted.clone();
    if let Some(v) = desk
        .prs
        .as_ref()
        .map(|s| s.viewer.trim())
        .filter(|v| !v.is_empty())
    {
        logins.push(v.to_string());
    }
    logins.extend(
        people
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(String::from),
    );
    logins
}

/// One of our PRs' author as the desk page shows it (read-only). A PR
/// that isn't ours has none: the page doesn't list it (card #625).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeskAuthor {
    pub pr: i64,
    /// The author card's id, "" when none yet.
    pub card: String,
    /// Its short ref (`#12`, `wind#4`), "" when none.
    pub card_ref: String,
    /// The agent behind it: the ledger's author, else the card's holder.
    pub agent: String,
}

/// The PR the desk holder is woken for (merge-desk.md §5, card #418): the
/// oldest of our PRs Waiting on desk, the queue's first such line, as the
/// desk page reads it: not a draft, not held, not behind or conflicting,
/// GitHub checks not pending or failed. `None` while the desk is off,
/// paused, unheld or a run is on. The old per-PR OK (`perPrOk`) no longer
/// gates it: nothing writes an OK any more (card #625).
pub fn waiting_pr<'a>(
    desk: &'a coord::Desk,
    verdicts: &[(i64, Verdict)],
) -> Option<(&'a coord::DeskPr, String)> {
    if !desk.on()
        || desk.settings.holder.is_empty()
        || desk.pause.as_ref().is_some_and(|p| p.paused)
        || desk.run.is_some()
    {
        return None;
    }
    let snap = desk.prs.as_ref()?;
    let mut ready: Vec<(&coord::DeskPr, String)> = snap
        .prs
        .iter()
        .filter_map(|p| {
            let card = match verdicts.iter().find(|(n, _)| *n == p.number)?.1 {
                Verdict::Card(ref m) | Verdict::NewCard(ref m) => m.task_id.clone(),
                Verdict::NotOurs => return None,
            };
            let waiting = !p.is_draft
                && !p.head_ref_oid.is_empty()
                && !desk.holds.contains_key(&p.number)
                && !matches!(p.merge_state_status.as_str(), "BEHIND" | "DIRTY")
                && p.mergeable != "CONFLICTING"
                && !matches!(p.checks.as_str(), "pending" | "failure");
            waiting.then_some((p, card))
        })
        .collect();
    ready.sort_by(|a, b| {
        (a.0.created_at.unix_milli_value(), a.0.number)
            .cmp(&(b.0.created_at.unix_milli_value(), b.0.number))
    });
    ready.into_iter().next()
}

/// The check lines as the holder's run text names them: a path-conditional
/// line says which paths it waits on.
pub fn check_lines(desk: &coord::Desk) -> Vec<String> {
    desk.settings
        .checks
        .iter()
        .map(|c| {
            if c.paths.is_empty() {
                format!("`{}`", c.command)
            } else {
                format!(
                    "`{}` (only if the diff touches {})",
                    c.command,
                    c.paths.join(", ")
                )
            }
        })
        .collect()
}

/// What says a repo is one of the person's (card #625): the person or a
/// member of their spaces opens PRs there. A repo an agent only cloned
/// (an upstream it reads) has none of these, so its desk is stray.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoEvidence {
    /// The person's and their members' GitHub logins.
    pub logins: Vec<String>,
    /// Repos (`owner/name`) one of them opened a PR in: the ledger's
    /// author rows for the person's agents, PR links on cards the person
    /// and members made (not the desk's own "PR #n" cards), and GitHub's
    /// search of each login's PRs.
    pub pr_repos: Vec<String>,
}

/// Whether `desk`'s repo is one of the person's: a login owns it, one of
/// them opened a PR there, or its latest snapshot lists a PR by one of
/// them (the snapshot replicates, so a device that can't search agrees).
pub fn our_repo(desk: &coord::Desk, ev: &RepoEvidence) -> bool {
    let Some(repo) = coord::github_repo(&desk.settings.repo_key) else {
        return false;
    };
    let login = |l: &str| {
        let l = l.trim();
        !l.is_empty() && ev.logins.iter().any(|x| x.trim().eq_ignore_ascii_case(l))
    };
    let owner = repo.split('/').next().unwrap_or_default();
    if login(owner) || ev.pr_repos.iter().any(|r| same_repo(r, &repo)) {
        return true;
    }
    desk.prs.as_ref().is_some_and(|s| {
        let viewer = s.viewer.trim();
        s.prs.iter().any(|p| {
            login(&p.author) || (!viewer.is_empty() && p.author.eq_ignore_ascii_case(viewer))
        })
    })
}
