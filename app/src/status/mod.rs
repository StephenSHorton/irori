//! Status (P11): the strip above every page and the OS glue behind it.
//!
//! The window side is the Rune port of the React app's
//! components/meters/* (the hardware meters strip and its panel, Claude
//! usage left) and updater/* (the update banner, the status line with
//! Check for updates, Keep this device up to date):
//! - [`banner`]: the update banner (Restart now / Later, remembering the
//!   version put off) over the meters strip, at the top of every page;
//! - [`update_section`]: Settings > App's Updates block, for the settings
//!   package's dialog;
//! - [`meters`], [`usage`]: lib/meters.ts and lib/usage.ts's logic.
//!
//! The shell side (what the Tauri shell had under app/src/shell/ on main)
//! is here too: [`updater`] and [`keep`] (the updater seam, its card and
//! Keep this device up to date's loop), [`tray`] and [`tray_text`] (the
//! tray menu). Launch at login's registration is app/src/login/.

pub mod keep;
pub mod meters;
mod sparkline;
pub mod tray;
pub mod tray_text;
pub mod updater;
pub mod usage;

use std::time::{SystemTime, UNIX_EPOCH};

use irori_window::events::EVENT_LAUNCH_AT_LOGIN;
use irori_window::meters::{EVENT_METERS_CHANGED, MetersSample};
use irori_window::update::{EVENT_UPDATE_STATUS, UpdateStatus};
use irori_window::usage::{EVENT_USAGE_CHANGED, UsageLimits};
use rune::tok::TextStyle;
use rune::{Align, Color, El, col, el, icon, oklch, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Placement, Spinner, popover};

use crate::ui::{Event, Frame, State, icons, theme};
use sparkline::{Max, sparkline};

/// The prefs key for Later: the update version the banner stays hidden
/// for (the React app's localStorage `irori.update.dismissed`).
pub const DISMISSED: &str = "update.dismissed";

#[derive(Default)]
pub struct Model {
    /// The last two minutes of readings, oldest first (crates/window's
    /// history, re-read on each `meters:changed`).
    pub meters: Vec<MetersSample>,
    pub usage: UsageLimits,
    /// The meters panel is open.
    pub meters_open: bool,

    /// The updater's state (`update:status`).
    pub update: UpdateStatus,
    /// The running version ("dev" in a dev build).
    pub version: String,
    /// This platform updates itself.
    pub supported: bool,
    /// A manual check is in flight.
    pub checking: bool,
    /// The last manual check found nothing newer.
    pub up_to_date: bool,
    /// Why the last Restart now failed ("" if it didn't).
    pub restart_error: String,
    /// The version Later put off (prefs [`DISMISSED`]).
    pub dismissed: String,
    /// Keep this device up to date: None where the device doesn't update
    /// itself, or until it's read.
    pub keep: Option<bool>,
    pub keep_error: Option<String>,
    /// The live launch-at-login state, from `settings:launch-at-login`
    /// (the settings package reads its own; this one follows the tray).
    pub launch_at_login: Option<bool>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Status has no page of its own (its parts are [`banner`] and
/// [`update_section`]); this is both, for a page that wants them.
#[allow(dead_code)] // The package contract: no route shows it.
pub fn view(s: &State, f: &mut Frame) -> El {
    col().w_full().child(banner(s, f)).child(col().padding(20.0).child(update_section(s, f)))
}

pub fn on_event(s: &mut State, ev: &Event) {
    let Some(w) = s.w.clone() else { return };
    match ev.name.as_str() {
        "app:start" => {
            s.status.dismissed = s.prefs.get_or(DISMISSED, String::new());
            s.status.version = irori_roles::commands::version().to_string();
            s.status.meters = w.meters_history();
            s.status.usage = w.usage_limits();
            s.status.update = w.update_status();
            s.status.supported = w.update_supported();
            // Reads the setting's file: off the UI thread.
            s.spawn(|w| w.update_keep_up_to_date(), |s, k| s.status.keep = k);
        }
        EVENT_METERS_CHANGED => s.status.meters = w.meters_history(),
        EVENT_USAGE_CHANGED => s.status.usage = w.usage_limits(),
        EVENT_UPDATE_STATUS => {
            let st = parse_status(&ev.payload).unwrap_or_else(|| w.update_status());
            // Any move off idle makes a stale "Up to date" wrong.
            if st.phase != "idle" {
                s.status.up_to_date = false;
            }
            s.status.update = st;
        }
        EVENT_LAUNCH_AT_LOGIN => s.status.launch_at_login = ev.payload.as_bool(),
        _ => {}
    }
}

/// update:status's payload (UpdateStatus is Serialize only).
fn parse_status(v: &serde_json::Value) -> Option<UpdateStatus> {
    let o = v.as_object()?;
    let g = |k: &str| o.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    Some(UpdateStatus {
        phase: o.get("phase")?.as_str()?.to_string(),
        version: g("version"),
        notes: g("notes"),
        error: g("error"),
    })
}

/// The headless scene (`irori --snap x.png --scene status`): an update
/// ready, and the meters panel open on two minutes of readings.
pub fn scene(s: &mut State) {
    s.status.update = UpdateStatus {
        phase: "ready".into(),
        version: "0.9.0".into(),
        ..UpdateStatus::default()
    };
    s.status.supported = true;
    s.status.dismissed.clear();
    if s.status.meters.len() < 2 {
        s.status.meters = sample_meters(now_ms());
    }
    if !usage::has_usage(&s.status.usage) {
        s.status.usage = sample_usage(now_ms());
    }
    s.status.meters_open = true;
}

// ---- actions ----------------------------------------------------------------

/// Restart now: launches the installer and quits. The updater goes back
/// to ready if the installer can't start; the reason shows.
fn restart(s: &mut State) {
    s.status.restart_error.clear();
    s.spawn(
        |w| w.update_restart(),
        |s, r| {
            if let Err(e) = r {
                s.status.restart_error = e;
            }
        },
    );
}

/// Later: hide the banner for this version; it installs on quit anyway.
fn dismiss(s: &mut State) {
    let v = s.status.update.version.clone();
    s.prefs.set(DISMISSED, &v);
    s.status.dismissed = v;
}

/// Check for updates: checks and downloads, never installs.
fn check(s: &mut State) {
    s.status.checking = true;
    s.status.up_to_date = false;
    s.spawn(
        |w| w.update_check_and_download(),
        |s, st| {
            s.status.checking = false;
            // "Up to date" is no backend phase: a finished check that
            // found nothing lands on idle.
            s.status.up_to_date = st.phase == "idle";
            s.status.update = st;
        },
    );
}

fn set_keep(s: &mut State, on: bool) {
    // The switch moves at once; the reply settles it.
    s.status.keep = Some(on);
    s.spawn(
        move |w| match w.update_set_keep_up_to_date(on) {
            Ok(v) => (v, None),
            Err(e) => (w.update_keep_up_to_date(), Some(e)),
        },
        |s, (v, e)| {
            s.status.keep = v;
            s.status.keep_error = e;
        },
    );
}

fn dismissed(s: &State) -> bool {
    !s.status.update.version.is_empty() && s.status.dismissed == s.status.update.version
}

fn vtag(v: &str) -> String {
    if v.is_empty() { String::new() } else { format!("v{v}") }
}

// ---- the banner -------------------------------------------------------------

/// The strip above every page: the update banner (while one is
/// downloading, installing or ready and not put off), then the meters.
pub fn banner(s: &State, f: &mut Frame) -> El {
    let mut c = col().key("status-banner").w_full();
    if let Some(u) = update_banner(s, f) {
        c = c.child(u);
    }
    c.child(meters_bar(s, f))
}

fn small(f: &Frame) -> TextStyle {
    f.k.ty.text_small
}

fn update_banner(s: &State, f: &mut Frame) -> Option<El> {
    let k = f.k;
    let st = &s.status.update;
    let v = vtag(&st.version);
    let muted = k.c.text_muted;
    match st.phase.as_str() {
        "downloading" => Some(
            col()
                .key("status-update")
                .w_full()
                .px(20.0)
                .pt(10.0)
                .gap(6.0)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(icon(&icons::DOWNLOAD, 14.0).color(muted))
                        .child(text(format!("Downloading Irori {v}…")).font(small(f)).color(muted)),
                )
                .child(
                    el().key("status-update-sweep")
                        .w_full()
                        .h(4.0)
                        .fill(k.c.surface_weak)
                        .clip_soft(0.0)
                        // irori-sweep: a quarter-width segment crosses the
                        // track, then comes round from the left again.
                        .child(el().w_frac(0.25).h_full().fill(accent(f)).looping(rune::LoopSpec::slide(
                            1.4,
                            [-1.0, 0.0],
                            [4.0, 0.0],
                        ))),
                ),
        ),
        "installing" => Some(
            row()
                .key("status-update")
                .px(20.0)
                .pt(10.0)
                .gap(8.0)
                .center_y()
                .child(Spinner::new().size(14.0).build(k))
                .child(text("Restarting to update…").font(small(f)).color(muted)),
        ),
        "ready" if !dismissed(s) => {
            let mut words = col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(format!("Irori {v} is ready."))
                        .font(k.ty.ui_medium)
                        .color(k.c.text_strong),
                )
                .child(
                    text("Restart now to update, or keep working. It installs when you quit Irori.")
                        .font(small(f))
                        .color(muted)
                        .wrap(),
                );
            if !s.status.restart_error.is_empty() {
                words = words.child(
                    text("Couldn't start the installer. Try again, or quit Irori to install.")
                        .font(small(f))
                        .color(theme::status::failed())
                        .help(s.status.restart_error.clone())
                        .wrap(),
                );
            }
            let panel = row()
                .key("status-update-ready")
                .w_full()
                .px(16.0)
                .py(12.0)
                .gap(16.0)
                .center_y()
                .fill(k.c.surface_raised)
                .border(1.0, k.c.border_weak)
                .radius(k.radius.md)
                .label(format!("Irori {v} is ready to install"))
                .child(icon(&icons::DOWNLOAD, 20.0).color(accent(f)))
                .child(words)
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            Button::new("Restart now")
                                .icon(&icons::REFRESH_CW)
                                .variant(ButtonVariant::Contrast)
                                .control_size(ControlSize::Regular)
                                .help(format!("Restart Irori and install {v}"))
                                .on_press(restart)
                                .build(k)
                                .key("status-restart"),
                        )
                        .child(
                            Button::new("Later")
                                .variant(ButtonVariant::Ghost)
                                .control_size(ControlSize::Regular)
                                .help("Keep working; it installs when you quit Irori")
                                .on_press(dismiss)
                                .build(k)
                                .key("status-later"),
                        ),
                );
            Some(col().key("status-update").w_full().px(20.0).pt(12.0).child(panel))
        }
        _ => None,
    }
}

fn accent(f: &Frame) -> Color {
    f.k.c.icon_accent
}

// ---- the meters -------------------------------------------------------------

/// The meters' colours (design/irori-ds.css), Task Manager's hues.
struct Hues {
    cpu: Color,
    mem: Color,
    disk: Color,
    net: Color,
    gpu: Color,
}

fn hues(dark: bool) -> Hues {
    if dark {
        Hues {
            cpu: oklch(0.74, 0.13, 240.0),
            mem: oklch(0.72, 0.14, 285.0),
            disk: oklch(0.78, 0.15, 140.0),
            net: oklch(0.74, 0.15, 15.0),
            gpu: oklch(0.72, 0.17, 320.0),
        }
    } else {
        Hues {
            cpu: oklch(0.56, 0.15, 245.0),
            mem: oklch(0.5, 0.17, 285.0),
            disk: oklch(0.55, 0.15, 140.0),
            net: oklch(0.56, 0.18, 15.0),
            gpu: oklch(0.52, 0.19, 320.0),
        }
    }
}

const TNUM: &str = "'tnum' 1";

/// What shows at which width (the React header's container queries):
/// CPU and memory first, then Claude, the GPU, then network and disk.
struct Fit {
    strip: bool,
    claude: bool,
    gpu: bool,
    more: bool,
}

fn fit(width: f32) -> Fit {
    Fit {
        strip: width > 500.0,
        claude: width >= 600.0,
        gpu: width >= 760.0,
        more: width >= 1000.0,
    }
}

/// The bar the meters strip sits in, right-aligned over the page.
fn meters_bar(s: &State, f: &mut Frame) -> El {
    let width = f.cx.layout_size("main").map_or(1100.0, |w| w[0]);
    let mut bar = row().key("status-bar").w_full().h(34.0).px(12.0).center_y().child(el().grow(1.0));
    let fit = fit(width);
    if fit.strip {
        bar = bar.child(meters_strip(s, f, &fit));
    }
    bar
}

fn values(samples: &[MetersSample], f: impl Fn(&MetersSample) -> Option<f64>) -> Vec<Option<f64>> {
    samples.iter().map(f).collect()
}

/// One meter in the strip: its name, its tiny graph, its reading.
fn item(f: &Frame, key: &str, label: &str, color: Color, vals: &[Option<f64>], max: Max, value: String, wide: bool) -> El {
    let k = f.k;
    row()
        .key(format!("status-meter-{key}"))
        .gap(4.0)
        .center_y()
        .child(text(label.to_string()).font(small(f)).color(k.c.text_muted))
        .child(sparkline(vals, meters::RING, max, 28.0, 12.0, color))
        .child(
            text(value)
                .font(small(f).features(TNUM))
                .color(k.c.text_base)
                .align(Align::End)
                .w(if wide { 62.0 } else { 30.0 }),
        )
}

/// The strip (MetersStrip): CPU, memory, GPU, network, disk and Claude's
/// plan left; a press opens the panel.
fn meters_strip(s: &State, f: &mut Frame, fit: &Fit) -> El {
    let k = f.k;
    let h = hues(f.dark);
    let m = &s.status.meters;
    let last = m.last();
    let now = now_ms();
    let mut strip = row()
        .key("status-meters-trigger")
        .h(30.0)
        .px(8.0)
        .gap(12.0)
        .center_y()
        .radius(k.radius.sm)
        .hover(k.c.hover)
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .role("button")
        .label("Hardware meters and Claude usage left")
        .on_press(|s: &mut State| s.status.meters_open = !s.status.meters_open);
    if s.status.meters_open {
        strip = strip.fill(k.c.hover);
    }
    strip = strip
        .child(item(
            f,
            "cpu",
            "CPU",
            h.cpu,
            &values(m, |s| s.cpu.pct.map(f64::from)),
            Max::Fixed(100.0),
            meters::fmt_pct(last.and_then(|l| l.cpu.pct.map(f64::from))),
            false,
        ))
        .child(item(
            f,
            "mem",
            "Mem",
            h.mem,
            &values(m, |s| meters::mem_pct(&s.mem)),
            Max::Fixed(100.0),
            meters::fmt_pct(last.and_then(|l| meters::mem_pct(&l.mem))),
            false,
        ));
    if fit.gpu
        && let Some(g) = meters::pick_gpu(m)
    {
        let util = |s: &MetersSample| meters::gpu_by_name(s, &g).and_then(|x| x.util_pct).map(f64::from);
        strip = strip.child(
            item(f, "gpu", "GPU", h.gpu, &values(m, util), Max::Fixed(100.0), meters::fmt_pct(last.and_then(util)), false)
                .help(g.clone()),
        );
    }
    if fit.more
        && let Some(l) = last
    {
        strip = strip.child(item(
            f,
            "net",
            "Net",
            h.net,
            &values(m, |s| Some(meters::net_total(s))),
            Max::Auto,
            meters::fmt_bits(meters::net_total(l)),
            true,
        ));
    }
    if fit.more
        && let Some(d) = meters::pick_disk(m)
    {
        let active = |s: &MetersSample| meters::disk_by_name(s, &d).and_then(|x| x.active_pct).map(f64::from);
        strip = strip.child(
            item(f, "disk", "Disk", h.disk, &values(m, active), Max::Fixed(100.0), meters::fmt_pct(last.and_then(active)), false)
                .help(meters::disk_name(&d)),
        );
    }
    if fit.claude
        && let Some(line) = usage::strip_line(&s.status.usage, now)
    {
        strip = strip.child(
            row()
                .key("status-meter-claude")
                .gap(4.0)
                .center_y()
                .child(el().w(1.0).h(16.0).fill(k.c.border_weak).mr(8.0))
                .child(text("Claude").font(small(f)).color(k.c.text_muted))
                .child(text(line.text).font(small(f).features(TNUM)).color(k.c.text_base))
                .help(format!("Claude usage: {}", line.title)),
        );
    }
    if s.status.meters_open {
        meters_panel(s, f, now);
    }
    strip
}

/// The panel the strip opens (MetersPanel): Claude usage left first, then
/// this computer's last two minutes, one row per meter.
fn meters_panel(s: &State, f: &mut Frame, now: i64) {
    let k = f.k;
    let h = hues(f.dark);
    let m = &s.status.meters;
    let mut body = col().key("status-meters-panel").w(352.0).child(usage_section(s, f, now)).child(
        text("This computer")
            .font(k.ty.text_small_medium)
            .color(k.c.text_muted)
            .w_full()
            .px(12.0)
            .pt(10.0)
            .pb(6.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_weak),
    );
    match m.last() {
        None => {
            body = body.child(
                text("No reading yet.")
                    .font(small(f))
                    .color(k.c.text_muted)
                    .px(12.0)
                    .pb(12.0),
            );
        }
        Some(last) => {
            let mut rows = col().key("status-meters-rows").w_full().pb(6.0);
            rows = rows
                .child(panel_row(f, "cpu", h.cpu, &values(m, |s| s.cpu.pct.map(f64::from)), Max::Fixed(100.0), "CPU", vec![meters::fmt_cpu(&last.cpu)]))
                .child(panel_row(f, "mem", h.mem, &values(m, |s| meters::mem_pct(&s.mem)), Max::Fixed(100.0), "Memory", vec![meters::fmt_memory(&last.mem)]));
            for d in &last.disks {
                let vals = values(m, |s| meters::disk_by_name(s, &d.name).and_then(|x| x.active_pct).map(f64::from));
                rows = rows.child(panel_row(
                    f,
                    &format!("disk-{}", d.name),
                    h.disk,
                    &vals,
                    Max::Fixed(100.0),
                    &meters::disk_name(&d.name),
                    vec![
                        format!("{} active", meters::fmt_pct(d.active_pct.map(f64::from))),
                        format!(
                            "R: {} W: {}",
                            meters::fmt_byte_rate(d.read_bytes_per_s),
                            meters::fmt_byte_rate(d.write_bytes_per_s)
                        ),
                    ],
                ));
            }
            rows = rows.child(panel_row(
                f,
                "net",
                h.net,
                &values(m, |s| Some(meters::net_total(s))),
                Max::Auto,
                "Network",
                vec![meters::fmt_net(&last.net)],
            ));
            for (i, g) in last.gpus.iter().enumerate() {
                let vals = values(m, |s| meters::gpu_by_name(s, &g.name).and_then(|x| x.util_pct).map(f64::from));
                let line = match meters::fmt_gpu_memory(g) {
                    Some(mem) => format!("{} · {mem}", meters::fmt_gpu(g)),
                    None => meters::fmt_gpu(g),
                };
                rows = rows.child(panel_row(
                    f,
                    &format!("gpu-{}", g.name),
                    h.gpu,
                    &vals,
                    Max::Fixed(100.0),
                    &format!("GPU {i}"),
                    vec![g.name.clone(), line],
                ));
            }
            body = body.child(popover::fit_scroll(rows));
        }
    }
    let surface = popover::raised(k, col().min_h(0.0).child(body), k.radius.md);
    f.layer.push(rune_kit::scrim("status-meters-scrim", |s: &mut State| s.status.meters_open = false));
    f.layer.push(popover::anchored(k, surface, "status-meters-trigger", Placement::BottomEnd, 4.0));
}

/// One meter in the panel: its graph in a frame on the left, its name
/// and readings on the right.
fn panel_row(f: &Frame, key: &str, color: Color, vals: &[Option<f64>], max: Max, title: &str, lines: Vec<String>) -> El {
    let k = f.k;
    let mut words = col().min_w(0.0).shrink().child(
        text(title.to_string())
            .font(k.ty.ui_medium)
            .color(k.c.text_strong)
            .truncate(),
    );
    for l in lines {
        words = words.child(
            text(l.clone())
                .font(small(f).features(TNUM))
                .color(k.c.text_muted)
                .truncate()
                .help(l),
        );
    }
    row()
        .key(format!("status-row-{key}"))
        .w_full()
        .px(12.0)
        .py(4.0)
        .gap(12.0)
        .center_y()
        .child(
            col()
                .w(72.0)
                .h(48.0)
                .justify_end()
                .border(1.0, color.alpha(0.55))
                .fill(k.c.page.alpha(0.4))
                .child(sparkline(vals, meters::RING, max, 70.0, 46.0, color)),
        )
        .child(words)
}

/// How much of each plan window is left: a bar, when it starts over, and
/// when the reading was taken if that was a while ago.
fn usage_section(s: &State, f: &Frame, now: i64) -> El {
    let k = f.k;
    let u = &s.status.usage;
    let stale = usage::stale_line(u.observed_at_ms, now, usage::local);
    let mut head = row()
        .w_full()
        .justify_between()
        .child(text("Claude usage left").font(k.ty.text_small_medium).color(k.c.text_muted));
    if let Some(st) = stale.filter(|_| usage::has_usage(u)) {
        head = head.child(text(st).font(small(f).features(TNUM)).color(k.c.text_muted));
    }
    let mut sec = col().key("status-usage").w_full().padding(12.0).gap(8.0).child(head);
    if !usage::has_usage(u) {
        return sec.child(
            text("No reading yet: appears after a Claude Code reply on this computer")
                .font(small(f))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    }
    for w in usage::sort_windows(&u.windows) {
        let left = usage::percent_left(&w, now);
        let tone = if left <= 10 {
            theme::status::failed()
        } else if left <= 25 {
            theme::status::decide()
        } else {
            accent(f)
        };
        let mut one = col()
            .key(format!("status-usage-{}", w.kind))
            .w_full()
            .gap(4.0)
            .child(
                row()
                    .w_full()
                    .justify_between()
                    .child(text(usage::window_label(&w.kind)).font(k.ty.text_small_medium).color(k.c.text_strong))
                    .child(text(format!("{left}% left")).font(small(f).features(TNUM)).color(k.c.text_strong)),
            )
            .child(
                el().w_full()
                    .h(6.0)
                    .radius(3.0)
                    .fill(k.c.surface_weak)
                    .child(el().w_frac(left as f32 / 100.0).h_full().radius(3.0).fill(tone)),
            );
        if let Some(r) = usage::reset_line(w.resets_at_ms, now, usage::local) {
            one = one.child(text(r).font(small(f).features(TNUM)).color(k.c.text_muted));
        }
        sec = sec.child(one);
    }
    sec
}

// ---- Settings > App: Updates ----------------------------------------------------

/// The Updates block of Settings > App (the React AppPane's "Updates":
/// UpdateStatusLine, then the Keep this device up to date switch), in
/// Settings > App.
pub fn update_section(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let mut c = col()
        .key("status-updates")
        .w_full()
        .gap(4.0)
        .pt(12.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base)
        .child(text("Updates").font(k.ty.ui_medium).color(k.c.text_strong))
        .child(status_line(s, f));
    if let Some(on) = s.status.keep {
        // Laid out as Settings' other switches: words left, switch right.
        c = c.child(col().pt(8.0).child(crate::settings::toggle(
            f,
            "status-keep",
            "Keep this device up to date",
            Some(
                "Checks every few hours and restarts Irori when no agent is working. \
                 If that hasn't happened within a day, it asks you.",
            ),
            on,
            false,
            |s, v| set_keep(s, v),
        )));
        if let Some(e) = &s.status.keep_error {
            c = c.child(text(e.clone()).font(small(f)).color(theme::status::failed()).wrap());
        }
    }
    c
}

/// The version line (UpdateStatusLine): the running version plus Check
/// for updates, or Restart now when an update is ready.
pub fn status_line(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let st = &s.status.update;
    let muted = k.c.text_muted;
    let target = vtag(&st.version);
    let v = if s.status.version.is_empty() { "dev" } else { s.status.version.as_str() };
    let busy = s.status.checking || st.phase == "downloading" || st.phase == "checking";
    let line = row().key("status-line").gap(8.0).center_y().flex_wrap();
    let t = |x: String, c: Color| text(x).font(small(f)).color(c);
    if st.phase == "installing" {
        let to = if target.is_empty() { String::new() } else { format!(" to {target}") };
        return line
            .child(Spinner::new().size(14.0).build(k))
            .child(t(format!("Restarting to update{to}…"), muted));
    }
    let name = if v == "dev" { "Irori (dev build)".to_string() } else { format!("Irori v{v}") };
    let mut line = line.child(t(name, muted)).child(t("·".into(), muted.alpha(0.4)));
    let failed = |line: El, what: &str, why: &str| {
        line.child(
            row()
                .gap(4.0)
                .center_y()
                .help(why.to_string())
                .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(theme::status::failed()))
                .child(t(what.to_string(), theme::status::failed())),
        )
    };
    if !s.status.supported {
        return line.child(t("Updates are not supported on this platform yet".into(), muted));
    }
    if st.phase == "ready" {
        line = line
            .child(
                row()
                    .gap(4.0)
                    .center_y()
                    .child(icon(&icons::DOWNLOAD, 14.0).color(accent(f)))
                    .child(t(format!("{target} is ready"), accent(f))),
            )
            .child(
                Button::new("Restart now")
                    .icon(&icons::REFRESH_CW)
                    .control_size(ControlSize::Small)
                    .help(format!("Restart Irori and install {target}"))
                    .on_press(restart)
                    .build(k)
                    .key("status-line-restart"),
            );
        if !s.status.restart_error.is_empty() {
            line = failed(line, "Restart failed", &s.status.restart_error);
        }
        return line;
    }
    let label = if st.phase == "downloading" {
        format!("Downloading {target}…")
    } else if s.status.checking || st.phase == "checking" {
        "Checking…".into()
    } else {
        "Check for updates".into()
    };
    line = line.child(
        Button::new(label)
            .icon(&icons::REFRESH_CW)
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .disabled(busy || v == "dev")
            .help(if v == "dev" { "Dev builds never update" } else { "Check GitHub Releases for a newer Irori" })
            .on_press(check)
            .build(k)
            .key("status-check"),
    );
    if s.status.up_to_date && !busy {
        line = line.child(
            row()
                .gap(4.0)
                .center_y()
                .child(icon(&icons::CIRCLE_CHECK, 14.0).color(accent(f)))
                .child(t("Up to date".into(), accent(f))),
        );
    }
    if st.phase == "error" {
        // A version means the check found one and the download or the
        // verify failed.
        let what = if st.version.is_empty() { "Check failed".to_string() } else { format!("Download of {target} failed") };
        line = failed(line, &what, &st.error);
    }
    line
}

// ---- the scene's readings ---------------------------------------------------------

/// Two minutes of a made-up machine (lib/meters.ts's sampleAt, cut down):
/// the scene's readings when the sampler hasn't read any yet.
fn sample_meters(now: i64) -> Vec<MetersSample> {
    use irori_window::meters::{MetersCpu, MetersDisk, MetersGpu, MetersMem, MetersNet};
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    (0..meters::RING as i64)
        .rev()
        .map(|i| {
            let t = (now - i * 2000) as f64 / 1000.0;
            let wave = |p: f64, ph: f64| ((t / p * std::f64::consts::TAU + ph).sin() + 1.0) / 2.0;
            let burst = if wave(23.0, 1.0) > 0.85 { 1.0 } else { 0.0 };
            let clamp = |n: f64| n.clamp(0.0, 100.0) as f32;
            MetersSample {
                at_ms: now - i * 2000,
                cpu: MetersCpu {
                    pct: Some(clamp(14.0 + 18.0 * wave(17.0, 0.0) + 30.0 * burst + 4.0 * wave(3.0, 0.5))),
                    ghz: Some((4.6 + 0.8 * wave(11.0, 0.0)) as f32),
                    cores: 16,
                },
                mem: MetersMem {
                    used_bytes: (61.6 * GIB * (0.41 + 0.05 * wave(60.0, 0.0))) as u64,
                    total_bytes: (61.6 * GIB) as u64,
                },
                disks: vec![MetersDisk {
                    name: "0 C:".into(),
                    active_pct: Some(clamp(2.0 + 30.0 * burst + 6.0 * wave(9.0, 0.0))),
                    read_bytes_per_s: (300_000.0 + 18e6 * burst) as u64,
                    write_bytes_per_s: (200_000.0 + 2e6 * wave(13.0, 0.0)) as u64,
                }],
                net: MetersNet {
                    tx_bits_per_s: (60e3 + 400e3 * wave(7.0, 2.0)) as u64,
                    rx_bits_per_s: (120e3 + 2.4e6 * wave(19.0, 0.0) + 30e6 * burst * 0.5) as u64,
                },
                gpus: vec![MetersGpu {
                    name: "Apple M-series GPU".into(),
                    integrated: true,
                    util_pct: Some(clamp(6.0 + 30.0 * wave(29.0, 3.0))),
                    temp_c: None,
                    mem_used_bytes: None,
                    mem_total_bytes: None,
                }],
            }
        })
        .collect()
}

/// The scene's Claude reading (lib/usage.ts's sampleUsage).
fn sample_usage(now: i64) -> UsageLimits {
    use irori_window::usage::UsageWindow;
    const MIN: i64 = 60_000;
    UsageLimits {
        observed_at_ms: now - 2 * MIN,
        windows: vec![
            UsageWindow {
                kind: "five_hour".into(),
                percent_used: 38.0,
                resets_at_ms: Some(now + 130 * MIN),
            },
            UsageWindow {
                kind: "seven_day".into(),
                percent_used: 61.0,
                resets_at_ms: Some(now + 3 * 24 * 60 * MIN),
            },
        ],
    }
}
