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
use crate::attachments::AttachmentInfo;
use crate::board::{AnswerResult, AskRepo, CommentResult, TaskResult, UploadFile};
use crate::boards::{BoardSettings, BoardSummary, SharePreview};
use crate::cards::{CardDetail, CardMessage, CardSummary, DonePage, IdleReport};
use crate::devices::{DeviceView, HomeServerView, PairStatus, RelayView};
use crate::events::{CaptureChange, Empty};
use crate::inbox::{InboxCounts, SendResult};
use crate::invites::InviteStatus;
use crate::mentions::{MentionCandidate, MentionOutcome};
use crate::roster::{GoneSession, RosterChange};
use crate::settings::{NotifyOptions, Target};
use crate::ts::{self, Typed};
use crate::update::UpdateStatus;
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
) -> R<Typed<Vec<coord::roster::AgentCard>, Vec<ts::AgentCard>>> {
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
) -> R<TaskResult> {
    w.start_thread(&text, &mentions, &[], &board_id)
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
) -> R<TaskResult> {
    blocking(&w, move |w| w.start_thread(&text, &mentions, &files, "")).await
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

/// Who a shared board's open cards go to (card #174).
#[tauri::command]
#[specta::specta]
pub async fn boards_set_open_cards(w: W<'_>, board_id: String, to: String) -> R<BoardSettings> {
    blocking(&w, move |w| {
        w.boards_set_open_cards(&board_id, &to)?;
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

/// Every command, plus the event payload types the frontend listens for.
pub(crate) fn builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            core_ready,
            spike_frames,
            roster_agents,
            roster_device,
            roster_version,
            roster_recent,
            roster_gone,
            roster_revive,
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
            inbox_allow_and_release,
            inbox_policy,
            inbox_set_policy,
            inbox_set_paused,
            board_cards,
            board_card,
            board_done_cards,
            board_cards_in,
            board_idle_report,
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
            boards_forget_now,
            boards_remove_member,
            boards_set_role,
            boards_forget,
            boards_share_preview,
            boards_share,
            boards_withdraw,
            boards_settings,
            boards_set_open_cards,
            boards_set_receive,
            boards_set_autonomy,
            boards_set_threads,
            boards_set_reviewer,
            boards_invite,
            boards_join_invite,
            boards_confirm_invite_words,
            boards_invite_status,
            boards_cancel_invite,
            devices_devices,
            devices_colors,
            devices_set_device_color,
            devices_rename_device,
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
            update_restart_to_update,
            voice_status,
            voice_download_model,
            voice_start,
            voice_stop,
            voice_open_settings,
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

/// S0b spike (#366, throwaway): streams `count` raw frames of `size` bytes
/// over a Channel of raw bytes, as fast as it can or every `interval_ms`.
/// Returns how long the sends took in microseconds.
#[tauri::command]
#[specta::specta]
pub async fn spike_frames(
    frames: tauri::ipc::Channel<RawFrame>,
    count: u32,
    size: u32,
    interval_ms: u32,
) -> R<f64> {
    let t0 = std::time::Instant::now();
    for i in 0..count {
        let mut buf = vec![(i & 0xff) as u8; size as usize];
        if size >= 4 {
            buf[..4].copy_from_slice(&i.to_le_bytes());
        }
        frames
            .send(RawFrame(buf))
            .map_err(|e| e.to_string())?;
        if interval_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(interval_ms as u64));
        }
    }
    Ok(t0.elapsed().as_micros() as f64)
}

/// S0b spike: one frame of raw bytes. Sent as `InvokeResponseBody::Raw`
/// (an ArrayBuffer in the webview, no JSON or base64); typed for the
/// bindings as `ArrayBuffer`, since `InvokeResponseBody` has no
/// `specta::Type`.
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
