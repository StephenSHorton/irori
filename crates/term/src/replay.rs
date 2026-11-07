//! Replay a recorded terminal stream into text (card #741).
//!
//! `claude logs <id>` prints the recent bytes of a background session's
//! terminal: a TUI repaint stream that homes the cursor (`ESC[H`), moves it
//! by absolute (`ESC[r;cH`) and relative (`ESC[nB`, `ESC[nC`, `ESC[nG`)
//! steps and paints over what was there. Deleting the escapes leaves the
//! text out of place and the `ESC[nC` gaps without their spaces, so the
//! bytes go through the same VT engine a live pane uses and come back as
//! what that screen showed, scrollback first.

use rsz_term::{Alacritty, Engine};

/// Scrollback lines kept while replaying, and so the most returned above
/// the screen.
const HISTORY: usize = 10_000;
/// The smallest screen a stream replays into.
const MIN_COLS: u16 = 160;
const MIN_ROWS: u16 = 24;
/// The largest, so a stray `ESC[99999B` can't ask for a huge grid.
const MAX_COLS: u16 = 1000;
const MAX_ROWS: u16 = 500;

/// The text `bytes` leave on a terminal: the scrollback, oldest first, then
/// the screen, with blank lines trimmed from both ends and trailing blanks
/// from every line.
pub fn replay_text(bytes: &[u8]) -> String {
    let (cols, rows) = geometry(bytes);
    let mut e = Alacritty::new(cols, rows, HISTORY);
    let mut events = Vec::new();
    e.feed(bytes, &mut events);
    // A synchronized update (DEC 2026) still open where the log was cut
    // holds back its last paint until it ends; end it.
    e.stop_sync(&mut events);
    let mut text = e.history_tail(HISTORY).join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&e.screen_text());
    text.trim_matches('\n').to_string()
}

/// A screen at least as big as the one the stream was drawn on. The stream
/// doesn't say its size, so it is read off the cursor moves: an absolute
/// `ESC[r;cH` or `ESC[r;cf`, and the relative moves a repaint makes from
/// home (`ESC[nB` down, `ESC[nC` right, `ESC[nG` to a column). Too small
/// and moves clamp at the edge, painting lines over each other; too big is
/// harmless, as the replay trims the blanks it leaves.
fn geometry(b: &[u8]) -> (u16, u16) {
    let (mut cols, mut rows) = (0u32, 0u32);
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] != 0x1b || b[i + 1] != b'[' {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        let mut params = [0u32; 2];
        let mut n = 0;
        while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';') {
            if b[j] == b';' {
                n += 1;
            } else if n < 2 {
                params[n] = params[n]
                    .saturating_mul(10)
                    .saturating_add(u32::from(b[j] - b'0'));
            }
            j += 1;
        }
        if j < b.len() {
            match b[j] {
                b'H' | b'f' => {
                    rows = rows.max(params[0]);
                    cols = cols.max(params[1]);
                }
                b'B' | b'E' => rows = rows.max(params[0].saturating_add(1)),
                b'C' => cols = cols.max(params[0].saturating_add(1)),
                b'G' | b'`' => cols = cols.max(params[0]),
                b'd' => rows = rows.max(params[0]),
                _ => {}
            }
        }
        i = j;
    }
    // Room for the text after the furthest move.
    let cols = cols
        .saturating_add(80)
        .clamp(MIN_COLS as u32, MAX_COLS as u32) as u16;
    let rows = rows.clamp(MIN_ROWS as u32, MAX_ROWS as u32) as u16;
    (cols, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_gaps_keep_their_spaces() {
        // The bug in #740's screenshot: ESC[nC skips cells, which strip_ansi
        // turned into nothing.
        let s = b"\x1b[H\rClaude\x1b[3CCode\x1b[20Gv2";
        assert_eq!(replay_text(s), "Claude   Code      v2");
    }

    #[test]
    fn repaints_leave_only_the_last_paint() {
        let s = b"\x1b[H\r\x1b[2BWorking\xe2\x80\xa6 old text\x1b[3;1HDone\x1b[K\x1b[H\rtop";
        assert_eq!(replay_text(s), "top\n\nDone");
    }

    #[test]
    fn bottom_rows_land_where_they_were_drawn() {
        // A 50-row session parks its cursor on row 50; a smaller replay
        // screen would clamp these onto one line.
        let s = b"\x1b[48;1Hprompt\x1b[50;1Hstatus\x1b[H\rhead";
        let text = replay_text(s);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 50);
        assert_eq!(lines[0], "head");
        assert_eq!(lines[47], "prompt");
        assert_eq!(lines[49], "status");
    }

    #[test]
    fn wide_moves_are_not_clipped() {
        let s = b"\x1b[H\r\x1b[300Cfar";
        assert_eq!(replay_text(s).trim_start(), "far");
        assert_eq!(replay_text(s).len(), 303);
    }

    #[test]
    fn lines_that_scroll_off_come_first() {
        let mut s = Vec::new();
        for n in 0..30 {
            s.extend_from_slice(format!("line {n}\r\n").as_bytes());
        }
        let text = replay_text(&s);
        let want: Vec<String> = (0..30).map(|n| format!("line {n}")).collect();
        assert_eq!(text, want.join("\n"));
    }

    #[test]
    fn colours_and_an_open_sync_are_no_trouble() {
        let s = b"\x1b[?2026h\x1b[H\r\x1b[38;2;215;119;87mhi\x1b[39m \x1b[1mthere\x1b[22m";
        assert_eq!(replay_text(s), "hi there");
    }

    #[test]
    fn huge_moves_are_capped() {
        assert_eq!(
            geometry(b"\x1b[99999999999;99999999999H"),
            (MAX_COLS, MAX_ROWS)
        );
        assert_eq!(geometry(b"plain"), (MIN_COLS, MIN_ROWS));
    }

    /// A real `claude logs` capture (a background session's last ~100 KB,
    /// every ASCII letter scrubbed to lorem ipsum, escapes kept) replays to
    /// the screen Attach showed. `claude-logs.txt` was read by eye against
    /// the session: lines in order, gaps kept, the newest prompt last. Rows
    /// with only a few cells are ones the session painted before the ring
    /// `claude logs` prints began; their other cells aren't in the stream.
    #[test]
    fn a_captured_repaint_stream_replays_to_its_screen() {
        let got = replay_text(include_bytes!("../testdata/claude-logs.bin"));
        let want = include_str!("../testdata/claude-logs.txt");
        assert_eq!(got, want.trim_end_matches('\n'));
    }
}
