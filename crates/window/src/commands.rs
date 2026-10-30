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
use crate::cards::{CardDetail, CardMessage, CardSummary, DonePage};
use crate::devices::{DeviceView, HomeServerView, PairStatus, RelayView};
use crate::events::{CaptureChange, Empty};
use crate::inbox::InboxCounts;
use crate::invites::InviteStatus;
use crate::roster::{GoneSession, RosterChange};
use crate::settings::{NotifyOptions, Target};
use crate::ts::{self, Typed};
use crate::update::UpdateStatus;

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
    Ok(Typed::new(w.core.device().clone()))
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
) -> R<Typed<inbox::Message, ts::Message>> {
    w.inbox_send(&to, &repo, &text).map(Typed::new)
}

#[tauri::command]
#[specta::specta]
pub async fn inbox_reply(w: W<'_>, delivery_id: String, text: String) -> R<()> {
    w.inbox_reply(&delivery_id, &text)
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

#[tauri::command]
#[specta::specta]
pub async fn board_message(w: W<'_>, task_id: String, msg_id: String) -> R<CardMessage> {
    w.message(&task_id, &msg_id)
}

#[tauri::command]
#[specta::specta]
pub async fn board_comment(w: W<'_>, id: String, text: String, tell: String) -> R<CommentResult> {
    w.comment(&id, &text, &tell)
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
) -> R<AnswerResult> {
    w.answer(&task_id, &blocker_id, option, &text, wake)
}

#[tauri::command]
#[specta::specta]
pub async fn board_resolve_blocker(
    w: W<'_>,
    task_id: String,
    blocker_id: String,
    note: String,
) -> R<AnswerResult> {
    w.resolve_blocker(&task_id, &blocker_id, &note)
}

#[tauri::command]
#[specta::specta]
pub async fn board_ask(w: W<'_>, text: String, repo_key: String, wake: bool) -> R<TaskResult> {
    w.ask(&text, &repo_key, wake, &[])
}

#[tauri::command]
#[specta::specta]
pub async fn board_ask_with_files(
    w: W<'_>,
    text: String,
    repo_key: String,
    wake: bool,
    files: Vec<UploadFile>,
) -> R<TaskResult> {
    blocking(&w, move |w| w.ask(&text, &repo_key, wake, &files)).await
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
    w.boards_leave(&board_id)
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

/// Every command, plus the event payload types the frontend listens for.
pub(crate) fn builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
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
            board_message,
            board_comment,
            board_answer,
            board_resolve_blocker,
            board_ask,
            board_ask_with_files,
            board_ask_again,
            board_ask_repos,
            board_close_ask,
            board_close,
            board_delete,
            board_pin,
            board_move_pin,
            board_reopen,
            boards_list,
            boards_create,
            boards_rename,
            boards_leave,
            boards_forget,
            boards_share_preview,
            boards_share,
            boards_withdraw,
            boards_settings,
            boards_set_receive,
            boards_set_autonomy,
            boards_invite,
            boards_join_invite,
            boards_confirm_invite_words,
            boards_invite_status,
            boards_cancel_invite,
            devices_devices,
            devices_colors,
            devices_set_device_color,
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
            devices_relay,
            devices_set_relay,
            settings_get_notifications,
            settings_set_notifications,
            settings_test_notification,
            settings_get_launch_at_login,
            settings_set_launch_at_login,
            attachments_info,
            attachments_fetch,
            attachments_save,
            attachments_open,
            update_get_status,
            update_get_version,
            update_supported,
            update_check_and_download,
            update_restart_to_update,
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
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        .dangerously_cast_bigints_to_number()
        .disable_serde_phases()
}
