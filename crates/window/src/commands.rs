//! The Tauri commands: one per service method the window calls (surface.md
//! 2.9), named `<service>_<method>`, each a thin wrapper over [`Window`].
//! Arguments arrive by name (camelCase of these parameter names); an error
//! rejects the frontend's promise with its text (tauri-specta's `Throw`).
//!
//! Every command is async, so it runs on the runtime, not the main
//! thread; a slow or blocking one (Revive, a file fetch or upload, the
//! pairing flow, the home server, Remove device, an update download) runs
//! on a blocking thread.

use std::sync::Arc;

use serde::Deserialize;
use specta::Type;
use tauri::State;

use crate::Window;
use crate::agents::{AgentsChange, AgentsSettings, RunningAgent};
use crate::attachments::AttachmentInfo;
use crate::board::{AnswerResult, AskRepo, CommentResult, TaskResult, UploadFile};
use crate::boards::{BoardSettings, BoardSummary, SharePreview};
use crate::cards::{CardDetail, CardMessage, CardSummary, DonePage, IdleReport};
use crate::desk::{DeskSettings, DeskView};
use crate::devices::{DeviceView, HomeServerView, PairStatus, RelayView};
use crate::events::{CaptureChange, Empty};
use crate::inbox::{InboxCounts, SendResult};
use crate::invites::InviteStatus;
use crate::mentions::{MentionCandidate, MentionOutcome};
use crate::roster::{GoneSession, RosterChange};
use crate::settings::{CardLook, NotifyOptions, Target};
use crate::talk::{TalkConversation, TalkMemberRef, TalkPost};
use crate::ts::{self, Typed};
use crate::update::{KeepUpToDate, UpdateStatus};
use crate::voice::{VoiceDownload, VoiceEvent, VoiceStatus};

type W<'a> = State<'a, Arc<Window>>;
type R<T> = Result<T, String>;

/// Runs `f` on a blocking thread.
async fn blocking<T: Send + 'static>(
    w: &Arc<Window>,
    f: impl FnOnce(&Window) -> R<T> + Send + 'static,
) -> R<T> {
    let w = w.clone();
    tauri::async_runtime::spawn_blocking(move || f(&w))
        .await
        .map_err(|e| e.to_string())?
}

/// The wake policy as the window sends it.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct PolicyInput {
    #[specta(type = ts::Mode)]
    pub mode: String,
    #[serde(default)]
    pub block: Vec<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub paused: bool,
}

// ---- RosterService ---------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn roster_agents(
    w: W<'_>,
) -> R<Typed<Vec<crate::roster::RosterAgent>, Vec<ts::AgentCard>>> {
    Ok(Typed::new(w.agents()))
}

#[tauri::command]
#[specta::specta]
pub async fn roster_device(w: W<'_>) -> R<Typed<irori_identity::DeviceJson, ts::Identity>> {
    // The header shows this device by the person's name for it (#184).
    let mut d = w.core.device().clone();
    let label = w.core.device_label(&d.id);
    if !label.is_empty() {
        d.name = label;
    }
    Ok(Typed::new(d))
}

#[tauri::command]
#[specta::specta]
pub async fn roster_version(w: W<'_>) -> R<String> {
    Ok(w.version())
}

#[tauri::command]
#[specta::specta]
pub async fn roster_recent(w: W<'_>, limit: i64) -> R<Typed<Vec<store::Event>, Vec<ts::Event>>> {
    w.recent(limit).map(Typed::new)
}

#[tauri::command]
#[specta::specta]
pub async fn roster_gone(w: W<'_>, limit: i64) -> R<Vec<GoneSession>> {
    w.gone(limit)
}

#[tauri::command]
#[specta::specta]
pub async fn roster_revive(w: W<'_>, session_id: String) -> R<String> {
    blocking(&w, move |w| w.revive(&session_id)).await
}

/// A check slot held now (card #437): the run in it (check.sh's name for
/// itself) and when its lease ends unless the run renews it.
#[derive(Debug, Clone, serde::Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckSlot {
    pub resource: String,
    pub run: String,
    pub until: String,
}

/// This device's check slots held now, by `scripts/check.sh` runs.
#[tauri::command]
#[specta::specta]
pub async fn roster_check_slots(w: W<'_>) -> R<Vec<CheckSlot>> {
    let dev = w.core.device_id().to_string();
    let mut out = Vec::new();
    for l in w.core.coord().leases() {
        let Some(run) = l
            .holder
            .strip_prefix(&format!("{dev}/{}", coord::RUN_HOLDER))
            .filter(|_| l.resource.starts_with("check/"))
        else {
            continue;
        };
        out.push(CheckSlot {
            resource: l.resource.clone(),
            run: run.to_string(),
            until: l.until.format_rfc3339(),
        });
    }
    Ok(out)
}

// ---- InboxService ----------------------------------------------------------------

type Deliveries = Typed<Vec<inbox::Delivery>, Vec<ts::Delivery>>;

#[tauri::command]
#[specta::specta]
pub async fn inbox_list(w: W<'_>, limit: i64) -> R<Deliveries> {
    Ok(Typed::new(w.inbox_list(limit)))
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_held(w: W<'_>) -> R<Deliveries> {
    Ok(Typed::new(w.inbox_held()))
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_held_history(w: W<'_>, limit: i64) -> R<Deliveries> {
    Ok(Typed::new(w.inbox_held_history(limit)))
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_counts(w: W<'_>) -> R<InboxCounts> {
    Ok(w.inbox_counts())
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_get(w: W<'_>, delivery_id: String) -> R<Typed<inbox::Delivery, ts::Delivery>> {
    w.inbox_get(&delivery_id).map(Typed::new)
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_send(
    w: W<'_>,
    to: String,
    repo: String,
    text: String,
    mentions: Vec<ts::Mention>,
) -> R<SendResult> {
    w.inbox_send(&to, &repo, &text, &mentions)
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_reply(
    w: W<'_>,
    delivery_id: String,
    text: String,
    mentions: Vec<ts::Mention>,
) -> R<Vec<MentionOutcome>> {
    w.inbox_reply(&delivery_id, &text, &mentions)
}

/// A message Reply with files (card #195).
#[tauri::command]
#[specta::specta]
pub async fn inbox_reply_with_files(
    w: W<'_>,
    delivery_id: String,
    text: String,
    mentions: Vec<ts::Mention>,
    files: Vec<UploadFile>,
) -> R<Vec<MentionOutcome>> {
    blocking(&w, move |w| {
        w.inbox_reply_files(&delivery_id, &text, &mentions, &files)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_mark_read(w: W<'_>, delivery_id: String) -> R<()> {
    w.inbox_mark_read(&delivery_id)
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_release(w: W<'_>, delivery_id: String) -> R<()> {
    w.inbox_release(&delivery_id)
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_dismiss(w: W<'_>, delivery_id: String) -> R<()> {
    w.inbox_dismiss(&delivery_id)
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_start_agent(w: W<'_>, delivery_id: String) -> R<ts::StartedAgent> {
    blocking(&w, move |w| w.inbox_start_agent(&delivery_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_repo_names(w: W<'_>) -> R<Vec<ts::RepoName>> {
    Ok(w.inbox_repo_names())
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_allow_and_release(w: W<'_>, delivery_id: String) -> R<()> {
    w.inbox_allow_and_release(&delivery_id)
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_policy(w: W<'_>) -> R<Typed<inbox::Policy, ts::Policy>> {
    Ok(Typed::new(w.inbox_policy()))
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_set_policy(w: W<'_>, p: PolicyInput) -> R<()> {
    w.inbox_set_policy(inbox::Policy {
        mode: p.mode,
        block: p.block,
        allow: p.allow,
        paused: p.paused,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_set_paused(w: W<'_>, paused: bool) -> R<()> {
    w.inbox_set_paused(paused)
}

// ---- BoardService ----------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn board_cards(w: W<'_>) -> R<Vec<CardSummary>> {
    Ok(w.cards())
}

#[tauri::command]
#[specta::specta]
pub async fn board_card(w: W<'_>, id: String) -> R<CardDetail> {
    w.card(&id)
}

/// Several cards in one call, each with its card or why not (card #468).
#[tauri::command]
#[specta::specta]
pub async fn board_cards_by_id(w: W<'_>, ids: Vec<String>) -> R<Vec<crate::cards::CardRead>> {
    blocking(&w, move |w| Ok(w.cards_by_id(&ids))).await
}

#[tauri::command]
#[specta::specta]
pub async fn board_done_cards(w: W<'_>, limit: i64, before: String) -> R<DonePage> {
    w.done_cards(limit, &before)
}

/// Home's idle line (card #341): idle agents and the open cards nobody
/// has been handed yet, with why.
#[tauri::command]
#[specta::specta]
pub async fn board_idle_report(w: W<'_>) -> R<IdleReport> {
    Ok(w.idle_report())
}

/// The person's notifications, newest first (card #377).
#[tauri::command]
#[specta::specta]
pub async fn notifications_list(w: W<'_>) -> R<Vec<crate::notifications::Notification>> {
    Ok(w.notifications())
}

/// How many notifications the person hasn't read (card #377).
#[tauri::command]
#[specta::specta]
pub async fn notifications_unread(w: W<'_>) -> R<u32> {
    Ok(w.unread_notifications())
}

/// Marks notifications read; no keys reads them all (card #377).
#[tauri::command]
#[specta::specta]
pub async fn notifications_read(w: W<'_>, keys: Vec<String>) -> R<()> {
    w.read_notifications(&keys)
}

/// Answers an agent's ask from its notification (card #377); refused once
/// it's answered or out of time.
#[tauri::command]
#[specta::specta]
pub async fn notifications_answer_ask(w: W<'_>, ask_id: String, choice: String) -> R<()> {
    w.answer_notification_ask(&ask_id, &choice)
}

/// One scope's live cards (card #161): "" every card, "own" the private
/// ones, or a board id.
#[tauri::command]
#[specta::specta]
pub async fn board_cards_in(w: W<'_>, scope: String) -> R<Vec<CardSummary>> {
    Ok(w.cards_in(&scope))
}

/// One page of one scope's closed cards (card #161).
#[tauri::command]
#[specta::specta]
pub async fn board_done_cards_in(
    w: W<'_>,
    limit: i64,
    before: String,
    scope: String,
) -> R<DonePage> {
    w.done_cards_in(limit, &before, &scope)
}

#[tauri::command]
#[specta::specta]
pub async fn board_clear_done_count(w: W<'_>, older_than_secs: i64, scope: String) -> R<i64> {
    Ok(w.clear_done_count(older_than_secs, &scope))
}

#[tauri::command]
#[specta::specta]
pub async fn board_clear_done(
    w: W<'_>,
    older_than_secs: i64,
    ids: Vec<String>,
    scope: String,
) -> R<i64> {
    w.clear_done(older_than_secs, &ids, &scope)
}

#[tauri::command]
#[specta::specta]
pub async fn board_message(w: W<'_>, task_id: String, msg_id: String) -> R<CardMessage> {
    w.message(&task_id, &msg_id)
}

#[tauri::command]
#[specta::specta]
pub async fn board_comment(
    w: W<'_>,
    id: String,
    text: String,
    tell: String,
    mentions: Vec<ts::Mention>,
) -> R<CommentResult> {
    w.comment(&id, &text, &tell, &mentions, &[])
}

/// Comment with files (card #124): pasted, dropped or picked in the
/// comment or thread Reply box.
#[tauri::command]
#[specta::specta]
pub async fn board_comment_with_files(
    w: W<'_>,
    id: String,
    text: String,
    tell: String,
    mentions: Vec<ts::Mention>,
    files: Vec<UploadFile>,
) -> R<CommentResult> {
    blocking(&w, move |w| w.comment(&id, &text, &tell, &mentions, &files)).await
}

#[tauri::command]
#[specta::specta]
pub async fn board_answer(
    w: W<'_>,
    task_id: String,
    blocker_id: String,
    option: i64,
    text: String,
    wake: bool,
    mentions: Vec<ts::Mention>,
) -> R<AnswerResult> {
    w.answer(&task_id, &blocker_id, option, &text, wake, &mentions)
}

/// An answer with files (card #195), stored off the bridge thread as a
/// comment's are.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn board_answer_with_files(
    w: W<'_>,
    task_id: String,
    blocker_id: String,
    option: i64,
    text: String,
    wake: bool,
    mentions: Vec<ts::Mention>,
    files: Vec<UploadFile>,
) -> R<AnswerResult> {
    blocking(&w, move |w| {
        w.answer_files(
            &task_id,
            &blocker_id,
            option,
            &text,
            wake,
            &mentions,
            &files,
        )
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn board_resolve_blocker(
    w: W<'_>,
    task_id: String,
    blocker_id: String,
    note: String,
    mentions: Vec<ts::Mention>,
) -> R<AnswerResult> {
    w.resolve_blocker(&task_id, &blocker_id, &note, &mentions)
}

#[tauri::command]
#[specta::specta]
pub async fn board_ask(
    w: W<'_>,
    text: String,
    repo_key: String,
    wake: bool,
    board_id: String,
    mentions: Vec<ts::Mention>,
) -> R<TaskResult> {
    w.ask(&text, &repo_key, wake, &[], &board_id, &mentions)
}

#[tauri::command]
#[specta::specta]
pub async fn board_ask_with_files(
    w: W<'_>,
    text: String,
    repo_key: String,
    wake: bool,
    files: Vec<UploadFile>,
    board_id: String,
    mentions: Vec<ts::Mention>,
) -> R<TaskResult> {
    blocking(&w, move |w| {
        w.ask(&text, &repo_key, wake, &files, &board_id, &mentions)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn board_mention_candidates(w: W<'_>) -> R<Vec<MentionCandidate>> {
    Ok(w.mention_candidates())
}

/// "Start an agent with this" on the person's comment (card #452).
#[tauri::command]
#[specta::specta]
pub async fn board_start_agent(w: W<'_>, id: String, note: String) -> R<()> {
    w.start_agent(&id, &note)
}

/// "Hand to an agent" on an item whose agent is gone (card #485).
#[tauri::command]
#[specta::specta]
pub async fn board_hand_on(w: W<'_>, id: String) -> R<String> {
    w.hand_on(&id)
}

#[tauri::command]
#[specta::specta]
pub async fn board_ask_again(w: W<'_>, id: String) -> R<()> {
    w.ask_again(&id)
}

#[tauri::command]
#[specta::specta]
pub async fn board_ask_repos(w: W<'_>) -> R<Vec<AskRepo>> {
    Ok(w.ask_repos())
}

#[tauri::command]
#[specta::specta]
pub async fn board_close_ask(
    w: W<'_>,
    id: String,
    note: String,
) -> R<Typed<coord::Task, ts::Task>> {
    w.close_ask(&id, &note).map(Typed::new)
}

#[tauri::command]
#[specta::specta]
pub async fn board_close(
    w: W<'_>,
    id: String,
    state: String,
    note: String,
) -> R<Typed<coord::Task, ts::Task>> {
    w.close_card(&id, &state, &note).map(Typed::new)
}

#[tauri::command]
#[specta::specta]
pub async fn board_delete(w: W<'_>, id: String) -> R<()> {
    w.delete_card(&id)
}

#[tauri::command]
#[specta::specta]
pub async fn board_pin(w: W<'_>, id: String, pinned: bool) -> R<()> {
    w.pin(&id, pinned)
}

/// Pause on a card (card #207): its holders are stopped and their tools
/// refused until Resume (`paused: false`). Returns who was told.
#[tauri::command]
#[specta::specta]
pub async fn board_pause(w: W<'_>, id: String, paused: bool, note: String) -> R<Vec<String>> {
    w.pause(&id, paused, &note)
}

/// Stop on an agent (card #207): cuts its running turn; `text` (empty: a
/// plain "stop and wait") is its next prompt.
#[tauri::command]
#[specta::specta]
pub async fn agent_stop(w: W<'_>, key: String, text: String) -> R<()> {
    w.agent_stop(&key, &text)
}

/// Wind down all (card #220): asks every agent (`scope` "this" device or
/// "all" devices) to save its place, hand back its cards and stop; with
/// `pause_after`, Pause all turns on once every agent has answered.
#[tauri::command]
#[specta::specta]
pub async fn wind_down_all(
    w: W<'_>,
    scope: String,
    pause_after: bool,
) -> R<crate::wind_down::WindDownStatus> {
    w.wind_down_all(&scope, pause_after)
}

/// The current Wind down all run, read fresh (empty id: none).
#[tauri::command]
#[specta::specta]
pub async fn wind_down_status(w: W<'_>) -> R<crate::wind_down::WindDownStatus> {
    Ok(w.wind_down_status())
}

/// Stop on a Wind down row: cuts that agent's turn with the wind-down's steps.
#[tauri::command]
#[specta::specta]
pub async fn wind_down_stop(w: W<'_>, key: String) -> R<()> {
    w.wind_down_stop(&key)
}

/// Stop and read on a message's Reply (card #207): the sender's running
/// turn is cut and the reply is its next prompt.
#[tauri::command]
#[specta::specta]
pub async fn inbox_reply_stop(w: W<'_>, id: String, text: String) -> R<Vec<MentionOutcome>> {
    w.inbox_reply_mode(&id, &text, &[], &[], ::inbox::mode::STOP)
}

#[tauri::command]
#[specta::specta]
pub async fn board_place(w: W<'_>, id: String, x: i64, y: i64) -> R<()> {
    w.place(&id, x, y)
}

/// Modify on the person's comment no agent read yet (card #105): holds it.
#[tauri::command]
#[specta::specta]
pub async fn board_comment_modify(w: W<'_>, id: String, comment_id: String) -> R<()> {
    w.comment_modify(&id, &comment_id)
}

/// Cancel on a comment held for editing: it goes out as it was.
#[tauri::command]
#[specta::specta]
pub async fn board_comment_modify_cancel(w: W<'_>, id: String, comment_id: String) -> R<()> {
    w.comment_modify_cancel(&id, &comment_id)
}

/// Save on a comment held for editing: its new text goes out.
#[tauri::command]
#[specta::specta]
pub async fn board_comment_modify_save(
    w: W<'_>,
    id: String,
    comment_id: String,
    text: String,
) -> R<()> {
    w.comment_modify_save(&id, &comment_id, &text).map(|_| ())
}

/// New card (card #116): a card on the person's own board, handed to one
/// free agent or to the agent picked.
#[tauri::command]
#[specta::specta]
pub async fn board_create(w: W<'_>, input: crate::new_card::NewCardInput) -> R<TaskResult> {
    // Its files (card #124) are stored off the bridge thread, as an ask's.
    blocking(&w, move |w| w.create_card(input)).await
}

/// The header box's message as a thread (card #116).
#[tauri::command]
#[specta::specta]
pub async fn board_start_thread(
    w: W<'_>,
    text: String,
    mentions: Vec<crate::ts::Mention>,
    board_id: String,
    agents_now: Option<bool>,
) -> R<TaskResult> {
    w.start_thread_with(&text, &mentions, &[], &board_id, agents_now)
}

/// A thread with files (card #124), as an ask takes them; your own
/// board only.
#[tauri::command]
#[specta::specta]
pub async fn board_start_thread_with_files(
    w: W<'_>,
    text: String,
    mentions: Vec<crate::ts::Mention>,
    files: Vec<UploadFile>,
    agents_now: Option<bool>,
) -> R<TaskResult> {
    blocking(&w, move |w| {
        w.start_thread_with(&text, &mentions, &files, "", agents_now)
    })
    .await
}

/// Turns an open thread's "Agents answer now" on or off (card #391).
#[tauri::command]
#[specta::specta]
pub async fn board_thread_agents_now(w: W<'_>, id: String, on: bool) -> R<()> {
    w.set_agents_now(&id, on)
}

/// What a #ref in text names (card #144), or null.
#[tauri::command]
#[specta::specta]
pub async fn board_card_ref(w: W<'_>, r: String) -> R<Option<crate::threads::RefTarget>> {
    Ok(w.card_ref(&r))
}

/// The Threads panel: open threads, then the newest resolved; your own
/// board's, or with `board_id` that shared board's (card #134).
#[tauri::command]
#[specta::specta]
pub async fn board_threads(w: W<'_>, board_id: String) -> R<Vec<crate::threads::ThreadSummary>> {
    Ok(w.threads(&board_id))
}

/// Resolve on a thread: the person is done with it.
#[tauri::command]
#[specta::specta]
pub async fn board_resolve_thread(w: W<'_>, id: String) -> R<()> {
    w.resolve_thread(&id)
}

/// A resolved thread is live again (card #169).
#[tauri::command]
#[specta::specta]
pub async fn board_reopen_thread(w: W<'_>, id: String) -> R<()> {
    w.reopen_thread(&id)
}

#[tauri::command]
#[specta::specta]
pub async fn board_move_pin(w: W<'_>, id: String, before: String) -> R<()> {
    w.move_pin(&id, &before)
}

#[tauri::command]
#[specta::specta]
pub async fn board_reopen(w: W<'_>, id: String) -> R<TaskResult> {
    w.reopen(&id)
}

// ---- BoardsService ---------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn boards_list(w: W<'_>) -> R<Vec<BoardSummary>> {
    Ok(w.boards_list())
}

/// Pins a board to the sidebar, or unpins it (card #135).
#[tauri::command]
#[specta::specta]
pub async fn boards_pin(w: W<'_>, board_id: String, pinned: bool) -> R<()> {
    w.boards_pin(&board_id, pinned)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_create(w: W<'_>, name: String, tag: String) -> R<BoardSummary> {
    w.boards_create(&name, &tag)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_add_to_space(
    w: W<'_>,
    space_id: String,
    name: String,
    tag: String,
) -> R<BoardSummary> {
    w.boards_add_to_space(&space_id, &name, &tag)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_rename(w: W<'_>, board_id: String, name: String) -> R<BoardSummary> {
    w.boards_rename(&board_id, &name)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_leave(w: W<'_>, board_id: String) -> R<()> {
    blocking(&w, move |w| w.boards_leave(&board_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn boards_forget_now(w: W<'_>, board_id: String) -> R<()> {
    blocking(&w, move |w| w.boards_forget_now(&board_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn boards_remove_member(
    w: W<'_>,
    board_id: String,
    person_id: String,
) -> R<BoardSettings> {
    blocking(&w, move |w| {
        w.boards_remove_member(&board_id, &person_id)?;
        w.boards_settings(&board_id)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn boards_set_role(
    w: W<'_>,
    board_id: String,
    person_id: String,
    role: String,
) -> R<BoardSettings> {
    w.boards_set_role(&board_id, &person_id, &role)?;
    w.boards_settings(&board_id)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_forget(w: W<'_>, board_id: String) -> R<()> {
    w.boards_forget(&board_id)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_share_preview(w: W<'_>, task_id: String, board_id: String) -> R<SharePreview> {
    w.boards_share_preview(&task_id, &board_id)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_share(w: W<'_>, task_id: String, board_id: String, hash: String) -> R<String> {
    w.boards_share(&task_id, &board_id, &hash)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_withdraw(w: W<'_>, task_id: String) -> R<String> {
    w.boards_withdraw(&task_id)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_transfer_owner(w: W<'_>, task_id: String, to: String) -> R<String> {
    w.boards_transfer_owner(&task_id, &to)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_settings(w: W<'_>, board_id: String) -> R<BoardSettings> {
    w.boards_settings(&board_id)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_set_receive(
    w: W<'_>,
    board_id: String,
    preset: String,
    people: Vec<String>,
) -> R<BoardSettings> {
    w.boards_set_receive(&board_id, &preset, people)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_set_autonomy(w: W<'_>, board_id: String, preset: String) -> R<BoardSettings> {
    w.boards_set_autonomy(&board_id, &preset)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_set_threads(w: W<'_>, board_id: String, mode: String) -> R<BoardSettings> {
    w.boards_set_threads(&board_id, &mode)
}

#[tauri::command]
#[specta::specta]
pub async fn boards_set_reviewer(
    w: W<'_>,
    board_id: String,
    agent: String,
    repo: String,
) -> R<BoardSettings> {
    w.boards_set_reviewer(&board_id, &agent, &repo)
}

// ---- Notes (card #401) ------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn notes_list(w: W<'_>, scope: String, removed: bool) -> R<Vec<crate::notes::NoteView>> {
    blocking(&w, move |w| w.notes_list(&scope, removed)).await
}

/// The sidebar's Notes count: notes waiting on the person's look (card #468).
#[tauri::command]
#[specta::specta]
pub async fn notes_waiting(w: W<'_>) -> R<u32> {
    blocking(&w, move |w| w.notes_waiting().map(|n| n as u32)).await
}

#[tauri::command]
#[specta::specta]
pub async fn note_new(w: W<'_>, note: crate::notes::NoteInput) -> R<crate::notes::NoteView> {
    blocking(&w, move |w| w.note_new(note)).await
}

#[tauri::command]
#[specta::specta]
pub async fn note_accept(w: W<'_>, id: String) -> R<crate::notes::NoteView> {
    blocking(&w, move |w| w.note_accept(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn note_edit(
    w: W<'_>,
    id: String,
    edit: crate::notes::NoteEditInput,
) -> R<crate::notes::NoteView> {
    blocking(&w, move |w| w.note_edit(&id, edit)).await
}

#[tauri::command]
#[specta::specta]
pub async fn note_remove(w: W<'_>, id: String, why: String) -> R<crate::notes::NoteView> {
    blocking(&w, move |w| w.note_remove(&id, &why)).await
}

/// A card's trust need, when its agent couldn't start because its repo
/// isn't trusted for agents (card #449).
#[tauri::command]
#[specta::specta]
pub async fn agents_trust_need(
    w: W<'_>,
    task_id: String,
) -> R<Option<crate::agents_trust::TrustNeed>> {
    blocking(&w, move |w| w.agents_trust_need(&task_id)).await
}

/// A board's agent permissions (card #449): `""` is the private board.
#[tauri::command]
#[specta::specta]
pub async fn agents_permissions(
    w: W<'_>,
    board_id: String,
) -> R<crate::agents_trust::AgentPermissions> {
    blocking(&w, move |w| Ok(w.agents_permissions(&board_id))).await
}

/// The person sets a board's agent permissions, from Settings › Agents.
#[tauri::command]
#[specta::specta]
pub async fn agents_permissions_set(
    w: W<'_>,
    permissions: crate::agents_trust::AgentPermissions,
) -> R<crate::agents_trust::AgentPermissions> {
    blocking(&w, move |w| w.agents_permissions_set(permissions)).await
}

/// The person's Trust and start: trusts the card's repo for agents on this
/// device, then starts its agent. One of the two paths that write trust.
#[tauri::command]
#[specta::specta]
pub async fn agents_trust_and_start(
    w: W<'_>,
    task_id: String,
) -> R<crate::agents_trust::TrustStarted> {
    blocking(&w, move |w| w.agents_trust_and_start(&task_id)).await
}

/// The person asks the device a card's agent must start on to trust its
/// repo there (part 11); that device asks them in its own window. Writes
/// no trust. The device's name.
#[tauri::command]
#[specta::specta]
pub async fn agents_trust_request(w: W<'_>, task_id: String) -> R<String> {
    blocking(&w, move |w| w.agents_trust_request(&task_id)).await
}

/// This device's repos and whether each is trusted for agents (card #456,
/// Settings › Agents).
#[tauri::command]
#[specta::specta]
pub async fn agents_trusted(w: W<'_>) -> R<Vec<crate::agents_trust::TrustedRepo>> {
    blocking(&w, move |w| w.agents_trusted()).await
}

/// The person's confirmed Trust… on one of this device's repos in
/// Settings › Agents (card #456); the other path that writes trust.
#[tauri::command]
#[specta::specta]
pub async fn agents_trust_repo(
    w: W<'_>,
    repo_key: String,
) -> R<Vec<crate::agents_trust::TrustedRepo>> {
    blocking(&w, move |w| w.agents_trust_repo(&repo_key)).await
}

// ---- Invites ----------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn boards_invite(w: W<'_>, board_id: String) -> R<InviteStatus> {
    blocking(&w, move |w| w.seams.invites.invite(&board_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn boards_join_invite(w: W<'_>, link: String) -> R<InviteStatus> {
    blocking(&w, move |w| w.seams.invites.join(&link)).await
}

#[tauri::command]
#[specta::specta]
pub async fn boards_confirm_invite_words(w: W<'_>, words: String) -> R<InviteStatus> {
    blocking(&w, move |w| w.seams.invites.confirm_words(&words)).await
}

#[tauri::command]
#[specta::specta]
pub async fn boards_invite_status(w: W<'_>) -> R<InviteStatus> {
    Ok(w.seams.invites.status())
}

#[tauri::command]
#[specta::specta]
pub async fn boards_cancel_invite(w: W<'_>) -> R<()> {
    w.seams.invites.cancel();
    Ok(())
}

// ---- DevicesService --------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn devices_devices(w: W<'_>) -> R<Vec<DeviceView>> {
    Ok(w.devices())
}

#[tauri::command]
#[specta::specta]
pub async fn devices_colors() -> R<Vec<String>> {
    Ok(coord::devcolor::PALETTE
        .iter()
        .map(|s| s.to_string())
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn devices_set_device_color(w: W<'_>, device_id: String, color: String) -> R<()> {
    w.set_device_color(&device_id, &color)
}

/// Agents' own faces by name key (card #407): open-bot-avatar JSON.
#[tauri::command]
#[specta::specta]
pub async fn devices_agent_avatars(w: W<'_>) -> R<std::collections::BTreeMap<String, String>> {
    Ok(w.core.agent_avatars().into_iter().collect())
}

#[tauri::command]
#[specta::specta]
pub async fn devices_person_color(w: W<'_>) -> R<String> {
    Ok(w.person_color())
}

#[tauri::command]
#[specta::specta]
pub async fn devices_set_person_color(w: W<'_>, color: String) -> R<()> {
    w.set_person_color(&color)
}

#[tauri::command]
#[specta::specta]
pub async fn devices_person_avatar(w: W<'_>) -> R<String> {
    Ok(w.person_avatar())
}

#[tauri::command]
#[specta::specta]
pub async fn devices_set_person_avatar(w: W<'_>, avatar: String) -> R<()> {
    w.set_person_avatar(&avatar)
}

/// Every work/personal side named on a device or a card (card #494).
#[tauri::command]
#[specta::specta]
pub async fn devices_side_names(w: W<'_>) -> R<Vec<String>> {
    Ok(w.side_names())
}

/// Pins the sides this device runs, the first its new cards' (card #494).
#[tauri::command]
#[specta::specta]
pub async fn devices_set_device_sides(w: W<'_>, sides: Vec<String>) -> R<Vec<String>> {
    w.set_device_sides(&sides)
}

#[tauri::command]
#[specta::specta]
pub async fn devices_rename_device(w: W<'_>, device_id: String, name: String) -> R<String> {
    w.rename_device(&device_id, &name)
}

#[tauri::command]
#[specta::specta]
pub async fn devices_add_device(w: W<'_>, role: String) -> R<PairStatus> {
    blocking(&w, move |w| w.seams.pairing.add(&role)).await
}

#[tauri::command]
#[specta::specta]
pub async fn devices_join_with_code(w: W<'_>, code: String, server: String) -> R<PairStatus> {
    blocking(&w, move |w| {
        w.seams.pairing.join(&code, crate::trim(&server))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn devices_confirm_words(w: W<'_>, words: String) -> R<()> {
    blocking(&w, move |w| w.seams.pairing.confirm_words(&words)).await
}

#[tauri::command]
#[specta::specta]
pub async fn devices_confirm_key(w: W<'_>, key: String) -> R<()> {
    blocking(&w, move |w| w.seams.pairing.confirm_key(&key)).await
}

#[tauri::command]
#[specta::specta]
pub async fn devices_pairing_status(w: W<'_>) -> R<PairStatus> {
    Ok(w.seams.pairing.status())
}

#[tauri::command]
#[specta::specta]
pub async fn devices_cancel_pairing(w: W<'_>) -> R<()> {
    w.seams.pairing.cancel();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn devices_remove_device(w: W<'_>, device_id: String) -> R<()> {
    blocking(&w, move |w| w.remove_device(&device_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn devices_reset_device(w: W<'_>) -> R<()> {
    w.reset_device()
}

#[tauri::command]
#[specta::specta]
pub async fn devices_home_server(w: W<'_>) -> R<HomeServerView> {
    Ok(w.home_server())
}

#[tauri::command]
#[specta::specta]
pub async fn devices_set_home_server(
    w: W<'_>,
    on: bool,
    outside: bool,
    public_addr: String,
) -> R<HomeServerView> {
    blocking(&w, move |w| w.set_home_server(on, outside, &public_addr)).await
}

#[tauri::command]
#[specta::specta]
pub async fn devices_set_home_server_awake(w: W<'_>, on: bool) -> R<HomeServerView> {
    blocking(&w, move |w| w.set_home_server_awake(on)).await
}

#[tauri::command]
#[specta::specta]
pub async fn devices_set_public_relays(w: W<'_>, on: bool) -> R<HomeServerView> {
    blocking(&w, move |w| w.set_public_relays(on)).await
}

#[tauri::command]
#[specta::specta]
pub async fn devices_relay(w: W<'_>) -> R<RelayView> {
    Ok(w.relay())
}

#[tauri::command]
#[specta::specta]
pub async fn devices_set_relay(w: W<'_>, credential_url: String) -> R<RelayView> {
    w.set_relay(&credential_url)
}

// ---- SettingsService -------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn settings_get_notifications(w: W<'_>) -> R<NotifyOptions> {
    Ok(w.get_notifications())
}

#[tauri::command]
#[specta::specta]
pub async fn settings_set_notifications(w: W<'_>, o: NotifyOptions) -> R<NotifyOptions> {
    w.set_notifications(o)
}

#[tauri::command]
#[specta::specta]
pub async fn settings_test_notification(w: W<'_>) -> R<()> {
    w.test_notification()
}

/// The desktop cards follow the window's appearance (card #527).
#[tauri::command]
#[specta::specta]
pub async fn settings_set_card_look(w: W<'_>, look: CardLook) -> R<()> {
    w.set_card_look(look)
}

#[tauri::command]
#[specta::specta]
pub async fn settings_get_launch_at_login(w: W<'_>) -> R<bool> {
    w.get_launch_at_login()
}

#[tauri::command]
#[specta::specta]
pub async fn settings_set_launch_at_login(w: W<'_>, enabled: bool) -> R<()> {
    w.set_launch_at_login(enabled)
}

/// The person's name (card #183).
#[tauri::command]
#[specta::specta]
pub async fn settings_get_your_name(w: W<'_>) -> R<String> {
    Ok(w.get_your_name())
}

/// The person renames themself; returns the name as kept.
#[tauri::command]
#[specta::specta]
pub async fn settings_set_your_name(w: W<'_>, name: String) -> R<String> {
    w.set_your_name(&name)
}

// ---- AttachmentsService ----------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn attachments_info(w: W<'_>, sha: String) -> R<AttachmentInfo> {
    w.attachment_info(&sha)
}

#[tauri::command]
#[specta::specta]
pub async fn attachments_fetch(w: W<'_>, sha: String) -> R<AttachmentInfo> {
    blocking(&w, move |w| w.attachment_fetch(&sha)).await
}

#[tauri::command]
#[specta::specta]
pub async fn attachments_save(w: W<'_>, sha: String) -> R<String> {
    blocking(&w, move |w| w.attachment_save(&sha)).await
}

#[tauri::command]
#[specta::specta]
pub async fn attachments_open(w: W<'_>, sha: String) -> R<()> {
    blocking(&w, move |w| w.attachment_open(&sha)).await
}

// ---- UpdateService (M8) ----------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn update_get_status(w: W<'_>) -> R<UpdateStatus> {
    Ok(w.update_status())
}

#[tauri::command]
#[specta::specta]
pub async fn update_get_version(w: W<'_>) -> R<String> {
    Ok(w.version())
}

#[tauri::command]
#[specta::specta]
pub async fn update_supported(w: W<'_>) -> R<bool> {
    Ok(w.update_supported())
}

#[tauri::command]
#[specta::specta]
pub async fn update_check_and_download(w: W<'_>) -> R<UpdateStatus> {
    // Waits out the whole download (up to the updater's deadline).
    blocking(&w, |w| Ok(w.update_check_and_download())).await
}

#[tauri::command]
#[specta::specta]
pub async fn update_restart_to_update(w: W<'_>) -> R<()> {
    w.update_restart()
}

#[tauri::command]
#[specta::specta]
pub async fn update_keep_up_to_date(w: W<'_>) -> R<KeepUpToDate> {
    Ok(w.update_keep_up_to_date())
}

#[tauri::command]
#[specta::specta]
pub async fn update_set_keep_up_to_date(w: W<'_>, on: bool) -> R<()> {
    w.update_set_keep_up_to_date(on)
}

/// Card #232: waits until the Core has started (the window opens first and
/// shows the opening screen); an error says why it couldn't start.
#[tauri::command]
#[specta::specta]
pub async fn core_ready(r: State<'_, Arc<crate::ready::Ready>>) -> R<()> {
    let r = r.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let res = r.wait();
        irori_core::log(&format!(
            "start: core_ready answered {} ms after launch",
            r.elapsed_ms()
        ));
        res
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- Voice input (#240) -----------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn voice_status(w: W<'_>) -> R<VoiceStatus> {
    Ok(w.voice_status())
}

#[tauri::command]
#[specta::specta]
pub async fn voice_download_model(w: W<'_>) -> R<()> {
    let w = w.inner().clone();
    w.voice_download_model().await
}

#[tauri::command]
#[specta::specta]
pub async fn voice_start(w: W<'_>) -> R<u32> {
    // Opening the microphone and compiling the dictation grammar block.
    let w = w.inner().clone();
    tauri::async_runtime::spawn_blocking(move || w.voice_start())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn voice_stop(w: W<'_>, session: u32) -> R<()> {
    blocking(&w, move |w| w.voice_stop(session)).await
}

#[tauri::command]
#[specta::specta]
pub async fn voice_open_settings(w: W<'_>, code: String) -> R<()> {
    blocking(&w, move |w| w.voice_open_settings(&code)).await
}

// ---- Settings › Agents (card #448) ------------------------------------------

/// A board's spawned-agent switch, model and effort (`""`: the private
/// board).
#[tauri::command]
#[specta::specta]
pub async fn agents_settings(w: W<'_>, board_id: String) -> R<AgentsSettings> {
    Ok(w.agents_settings(&board_id))
}

/// The person changes them: the app only, never a tool.
#[tauri::command]
#[specta::specta]
pub async fn agents_set(w: W<'_>, change: AgentsChange) -> R<AgentsSettings> {
    w.agents_set(change)
}

/// The person's "Offer it cards" on one of their terminal agents (card
/// #453): the app only, never a tool. Spawned agents are refused.
#[tauri::command]
#[specta::specta]
pub async fn agents_offer_set(w: W<'_>, key: String, on: bool) -> R<()> {
    blocking(&w, move |w| w.agents_offer_set(&key, on)).await
}

/// Every agent Irori started that is still running, or is starting.
#[tauri::command]
#[specta::specta]
pub async fn agents_running(w: W<'_>) -> R<Vec<RunningAgent>> {
    blocking(&w, move |w| Ok(w.agents_running())).await
}

/// The person ends an agent Irori started: its hand-off, then stop and rm.
#[tauri::command]
#[specta::specta]
pub async fn agents_end(w: W<'_>, key: String) -> R<()> {
    blocking(&w, move |w| w.agents_end(&key)).await
}

/// Logs on an agent Irori started (card #456): its recent output, read only.
#[tauri::command]
#[specta::specta]
pub async fn agents_logs(w: W<'_>, key: String) -> R<String> {
    blocking(&w, move |w| w.agents_logs(&key)).await
}

/// Attach on an agent Irori started (card #456): a tab in its card's
/// terminal running `claude attach`.
#[tauri::command]
#[specta::specta]
pub async fn agents_attach(w: W<'_>, key: String, cols: u16, rows: u16) -> R<term::TermInfo> {
    blocking(&w, move |w| w.agents_attach(&key, cols, rows)).await
}

// ---- merge desks (card #417) ----------------------------------------------

/// Every merge desk in the person's log.
#[tauri::command]
#[specta::specta]
pub async fn desk_list(w: W<'_>) -> R<Vec<DeskView>> {
    Ok(w.desks())
}

/// The person sets up or changes a repo's merge desk.
#[tauri::command]
#[specta::specta]
pub async fn desk_set(w: W<'_>, settings: DeskSettings) -> R<()> {
    w.desk_set(settings)
}

/// The person holds a desk's PR, or with `cleared` lifts the hold.
#[tauri::command]
#[specta::specta]
pub async fn desk_hold(w: W<'_>, repo_key: String, pr: i64, note: String, cleared: bool) -> R<()> {
    w.desk_hold(&repo_key, pr, &note, cleared)
}

/// The person pauses or resumes a desk.
#[tauri::command]
#[specta::specta]
pub async fn desk_pause(w: W<'_>, repo_key: String, paused: bool) -> R<()> {
    w.desk_pause(&repo_key, paused)
}

/// The person okays an Outside PR at one head.
#[tauri::command]
#[specta::specta]
pub async fn desk_ok(w: W<'_>, repo_key: String, pr: i64, head_sha: String) -> R<()> {
    w.desk_ok(&repo_key, pr, &head_sha)
}

// ---- talk (card #389) -----------------------------------------------------

/// Talk's list: channels, DMs and open threads, with unread counts.
#[tauri::command]
#[specta::specta]
pub async fn talk_conversations(w: W<'_>) -> R<Vec<TalkConversation>> {
    w.talk_conversations()
}

/// Up to `limit` posts before `before` (none: the newest), oldest first.
#[tauri::command]
#[specta::specta]
pub async fn talk_posts(
    w: W<'_>,
    conv_id: String,
    before: Option<String>,
    limit: i64,
) -> R<Vec<TalkPost>> {
    w.talk_posts(&conv_id, before.as_deref().filter(|b| !b.is_empty()), limit)
}

/// Posts as the person; returns the post's id (a thread's: the comment's).
#[tauri::command]
#[specta::specta]
pub async fn talk_post(
    w: W<'_>,
    conv_id: String,
    text: String,
    mentions: Vec<String>,
    agents_now: bool,
    thread_from: Option<String>,
) -> R<String> {
    w.talk_post(
        &conv_id,
        &text,
        &mentions,
        agents_now,
        thread_from.filter(|t| !t.is_empty()),
    )
}

/// The person asks about channel `conv_id`'s post `post_id` in a new
/// thread (card #454): it records the channel and its repo, and the
/// channel gets a pointer to it.
#[tauri::command]
#[specta::specta]
pub async fn talk_thread_from(
    w: W<'_>,
    conv_id: String,
    post_id: String,
    text: String,
) -> R<TaskResult> {
    w.talk_thread_from(&conv_id, &post_id, &text)
}

/// The person made card `task_id` from post `post_id` (card #390).
#[tauri::command]
#[specta::specta]
pub async fn talk_link_card(w: W<'_>, post_id: String, task_id: String) -> R<()> {
    w.talk_link_card(&post_id, &task_id)
}

/// The person has read `conv_id` up to post `up_to`.
#[tauri::command]
#[specta::specta]
pub async fn talk_read(w: W<'_>, conv_id: String, up_to: String) -> R<()> {
    w.talk_read(&conv_id, &up_to)
}

/// Makes a channel; returns its conv id. `space_id` puts it in that
/// space, where its members see it (card #392); none is Personal.
#[tauri::command]
#[specta::specta]
pub async fn talk_channel_create(
    w: W<'_>,
    name: String,
    topic: Option<String>,
    space_id: Option<String>,
) -> R<String> {
    w.talk_channel_create(&name, topic.as_deref(), space_id.as_deref())
}

/// The person's DM with an agent, opened if new; returns its conv id.
#[tauri::command]
#[specta::specta]
pub async fn talk_dm(w: W<'_>, member: TalkMemberRef) -> R<String> {
    w.talk_dm(&member)
}

#[tauri::command]
#[specta::specta]
pub async fn talk_join(w: W<'_>, conv_id: String, member: TalkMemberRef) -> R<()> {
    w.talk_join(&conv_id, &member)
}

#[tauri::command]
#[specta::specta]
pub async fn talk_leave(w: W<'_>, conv_id: String, member: TalkMemberRef) -> R<()> {
    w.talk_leave(&conv_id, &member)
}

/// Every command, plus the event payload types the frontend listens for.
pub(crate) fn builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            core_ready,
            roster_agents,
            roster_device,
            roster_version,
            roster_recent,
            roster_gone,
            roster_revive,
            roster_check_slots,
            inbox_list,
            inbox_held,
            inbox_held_history,
            inbox_counts,
            inbox_get,
            inbox_send,
            inbox_reply,
            inbox_mark_read,
            inbox_release,
            inbox_dismiss,
            inbox_start_agent,
            inbox_repo_names,
            inbox_allow_and_release,
            inbox_policy,
            inbox_set_policy,
            inbox_set_paused,
            board_cards,
            board_card,
            board_cards_by_id,
            board_done_cards,
            board_cards_in,
            board_idle_report,
            notifications_list,
            notifications_unread,
            notifications_read,
            notifications_answer_ask,
            board_done_cards_in,
            board_clear_done_count,
            board_clear_done,
            board_message,
            board_comment,
            board_comment_with_files,
            board_answer_with_files,
            inbox_reply_with_files,
            board_answer,
            board_resolve_blocker,
            board_ask,
            board_ask_with_files,
            board_ask_again,
            board_start_agent,
            board_hand_on,
            board_ask_repos,
            board_mention_candidates,
            board_close_ask,
            board_close,
            board_delete,
            board_pin,
            board_pause,
            agent_stop,
            wind_down_all,
            wind_down_status,
            wind_down_stop,
            inbox_reply_stop,
            board_place,
            board_create,
            board_start_thread,
            board_start_thread_with_files,
            board_threads,
            board_card_ref,
            board_resolve_thread,
            board_reopen_thread,
            board_thread_agents_now,
            board_comment_modify,
            board_comment_modify_cancel,
            board_comment_modify_save,
            board_move_pin,
            board_reopen,
            boards_list,
            boards_pin,
            boards_create,
            boards_rename,
            boards_leave,
            boards_add_to_space,
            boards_forget_now,
            boards_remove_member,
            boards_set_role,
            boards_forget,
            boards_share_preview,
            boards_share,
            boards_withdraw,
            boards_transfer_owner,
            boards_settings,
            boards_set_receive,
            boards_set_autonomy,
            boards_set_threads,
            boards_set_reviewer,
            notes_list,
            notes_waiting,
            note_new,
            note_accept,
            note_edit,
            note_remove,
            agents_trust_need,
            agents_trust_and_start,
            agents_trust_request,
            agents_trusted,
            agents_trust_repo,
            agents_permissions,
            agents_permissions_set,
            boards_invite,
            boards_join_invite,
            boards_confirm_invite_words,
            boards_invite_status,
            boards_cancel_invite,
            devices_devices,
            devices_colors,
            devices_set_device_color,
            devices_person_color,
            devices_agent_avatars,
            devices_set_person_color,
            devices_person_avatar,
            devices_set_person_avatar,
            devices_rename_device,
            devices_side_names,
            devices_set_device_sides,
            devices_add_device,
            devices_join_with_code,
            devices_confirm_words,
            devices_confirm_key,
            devices_pairing_status,
            devices_cancel_pairing,
            devices_remove_device,
            devices_reset_device,
            devices_home_server,
            devices_set_home_server,
            devices_set_home_server_awake,
            devices_set_public_relays,
            devices_relay,
            devices_set_relay,
            settings_get_notifications,
            settings_set_notifications,
            settings_test_notification,
            settings_set_card_look,
            settings_get_launch_at_login,
            settings_set_launch_at_login,
            settings_get_your_name,
            settings_set_your_name,
            attachments_info,
            attachments_fetch,
            attachments_save,
            attachments_open,
            update_get_status,
            update_get_version,
            update_supported,
            update_check_and_download,
            update_keep_up_to_date,
            update_set_keep_up_to_date,
            update_restart_to_update,
            voice_status,
            voice_download_model,
            voice_start,
            voice_stop,
            voice_open_settings,
            talk_conversations,
            talk_posts,
            talk_post,
            talk_link_card,
            talk_thread_from,
            talk_read,
            talk_channel_create,
            talk_dm,
            talk_join,
            talk_leave,
            term_open,
            term_attach,
            term_detach,
            term_write,
            term_paste,
            term_resize,
            term_scroll,
            term_cell_px,
            term_focus,
            term_redraw,
            term_ack,
            term_close,
            term_folders,
            term_list,
            agents_settings,
            agents_set,
            agents_offer_set,
            agents_end,
            agents_logs,
            agents_attach,
            agents_running,
            desk_list,
            desk_set,
            desk_hold,
            desk_pause,
            desk_ok,
        ])
        .typ::<RosterChange>()
        .typ::<CaptureChange>()
        .typ::<Empty>()
        .typ::<Target>()
        .typ::<ts::TaskState>()
        .typ::<ts::LinkKind>()
        .typ::<ts::State>()
        .typ::<ts::HoldReason>()
        .typ::<crate::cards::Holding>()
        .typ::<VoiceEvent>()
        .typ::<VoiceDownload>()
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        .dangerously_cast_bigints_to_number()
        .disable_serde_phases()
}

// ---- Terminals (card #369; docs/design/inline-terminals.md section 5) ----------

/// One terminal frame as raw bytes: `InvokeResponseBody::Raw`, an
/// ArrayBuffer in the webview with no JSON or base64. Frames never go
/// through events, which are coalesced (design section 5). The route is
/// S0b's (#366): `InvokeResponseBody` has no `specta::Type`, so this
/// newtype is typed as `ArrayBuffer` for the bindings.
pub struct RawFrame(pub Vec<u8>);

impl tauri::ipc::IpcResponse for RawFrame {
    fn body(self) -> tauri::Result<tauri::ipc::InvokeResponseBody> {
        Ok(tauri::ipc::InvokeResponseBody::Raw(self.0))
    }
}

impl specta::Type for RawFrame {
    fn definition(_: &mut specta::Types) -> specta::datatype::DataType {
        specta_typescript::define("ArrayBuffer").into()
    }
}

#[tauri::command]
#[specta::specta]
pub async fn term_open(
    w: W<'_>,
    scope: term::Scope,
    cols: u16,
    rows: u16,
    cwd: Option<String>,
) -> R<term::TermInfo> {
    blocking(&w, move |w| w.term_open(scope, cols, rows, cwd)).await
}

#[tauri::command]
#[specta::specta]
pub async fn term_attach(
    w: W<'_>,
    id: u32,
    frames: tauri::ipc::Channel<RawFrame>,
    msgs: tauri::ipc::Channel<term::TermMsg>,
) -> R<()> {
    let viewer = term::Viewer::new(
        move |b| frames.send(RawFrame(b)).is_ok(),
        move |m| msgs.send(m).is_ok(),
    );
    w.core.terms().attach(id, viewer)
}

#[tauri::command]
#[specta::specta]
pub async fn term_detach(w: W<'_>, id: u32) -> R<()> {
    w.core.terms().detach(id);
    Ok(())
}

// What the person types reaches the shell in the order typed: these are
// sync commands, which Tauri runs one at a time on the main thread. Each
// only queues bytes for the pane's actor. As async commands, every key was
// its own task, and on the PC a fast burst once arrived out of order
// (#369, kestrel).
#[tauri::command]
#[specta::specta]
pub fn term_write(w: W<'_>, id: u32, data: String) -> R<()> {
    w.core.terms().write(id, data.into_bytes())
}

#[tauri::command]
#[specta::specta]
pub fn term_paste(w: W<'_>, id: u32, text: String) -> R<()> {
    w.core.terms().paste(id, &text)
}

#[tauri::command]
#[specta::specta]
pub fn term_resize(w: W<'_>, id: u32, cols: u16, rows: u16) -> R<()> {
    w.core.terms().resize(id, cols, rows)
}

#[tauri::command]
#[specta::specta]
pub fn term_scroll(w: W<'_>, id: u32, delta: i32) -> R<()> {
    w.core.terms().scroll(id, delta)
}

#[tauri::command]
#[specta::specta]
pub async fn term_cell_px(w: W<'_>, id: u32, width: u16, height: u16) -> R<()> {
    w.core.terms().cell_px(id, width, height)
}

#[tauri::command]
#[specta::specta]
pub fn term_focus(w: W<'_>, id: u32, on: bool) -> R<()> {
    w.core.terms().focus(id, on)
}

#[tauri::command]
#[specta::specta]
pub async fn term_redraw(w: W<'_>, id: u32) -> R<()> {
    w.core.terms().redraw(id)
}

#[tauri::command]
#[specta::specta]
pub fn term_ack(w: W<'_>, id: u32, seq: u32) -> R<()> {
    w.core.terms().ack(id, seq)
}

#[tauri::command]
#[specta::specta]
pub async fn term_close(w: W<'_>, id: u32) -> R<()> {
    w.core.terms().close(id)
}

#[tauri::command]
#[specta::specta]
pub async fn term_list(w: W<'_>, card: String) -> R<Vec<term::TermInfo>> {
    Ok(w.term_list(&card))
}

#[tauri::command]
#[specta::specta]
pub async fn term_folders(w: W<'_>, card: String) -> R<Vec<crate::terms::Folder>> {
    blocking(&w, move |w| w.term_folders(&card)).await
}
