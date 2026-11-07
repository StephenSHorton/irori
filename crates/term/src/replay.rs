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
/// doesn't say its size, so this follows the cursor through it (absolute
/// and relative moves, printed text, CR, LF, tab, backspace) and takes the
/// furthest cell it reaches. Too small and the replay wraps long rows or
/// clamps moves at the edge, painting lines over each other; too big is
/// harmless, as the replay trims the blanks it leaves. So it errs big: a
/// non-ASCII character counts two cells when it may be wide, and every LF
/// moves down, though the session's own screen scrolled instead.
fn geometry(b: &[u8]) -> (u16, u16) {
    let (mut col, mut row) = (0u32, 0u32);
    // The furthest cell reached, as a count (one past its index).
    let (mut cols, mut rows) = (0u32, 0u32);
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        i += 1;
        match c {
            0x1b => {
                let Some(&k) = b.get(i) else { break };
                i += 1;
                match k {
                    b'[' => {
                        let (n, end) = csi(b, i);
                        i = end;
                        let Some(f) = n.final_byte else { continue };
                        if n.private {
                            continue;
                        }
                        let p0 = n.params[0].max(1);
                        match f {
                            b'H' | b'f' => {
                                row = p0 - 1;
                                col = n.params[1].max(1) - 1;
                            }
                            b'A' => row = row.saturating_sub(p0),
                            b'B' | b'e' => row = row.saturating_add(p0),
                            b'C' | b'a' => col = col.saturating_add(p0),
                            b'D' => col = col.saturating_sub(p0),
                            b'E' => (row, col) = (row.saturating_add(p0), 0),
                            b'F' => (row, col) = (row.saturating_sub(p0), 0),
                            b'G' | b'`' => col = p0 - 1,
                            b'd' => row = p0 - 1,
                            _ => continue,
                        }
                        cols = cols.max(col.saturating_add(1));
                        rows = rows.max(row.saturating_add(1));
                    }
                    // OSC, DCS, SOS, PM, APC: a string up to BEL or ST.
                    b']' | b'P' | b'X' | b'^' | b'_' => {
                        while i < b.len()
                            && b[i] != 0x07
                            && !(b[i] == 0x1b && b.get(i + 1) == Some(&b'\\'))
                        {
                            i += 1;
                        }
                        i += if b.get(i) == Some(&0x07) { 1 } else { 2 };
                    }
                    // ESC ( B and friends: intermediates, then a final byte.
                    0x20..=0x2f => {
                        while i < b.len() && (0x20..=0x2f).contains(&b[i]) {
                            i += 1;
                        }
                        i += 1;
                    }
                    _ => {}
                }
            }
            b'\r' => col = 0,
            b'\n' | 0x0b | 0x0c => row = row.saturating_add(1),
            0x08 => col = col.saturating_sub(1),
            b'\t' => col = (col / 8 + 1).saturating_mul(8),
            0x00..=0x1f | 0x7f => {}
            // A UTF-8 continuation byte belongs to the character before.
            0x80..=0xbf => {}
            _ => {
                col = col.saturating_add(if c < 0x80 { 1 } else { 2 });
                cols = cols.max(col);
                rows = rows.max(row.saturating_add(1));
            }
        }
    }
    let cols = cols.clamp(MIN_COLS as u32, MAX_COLS as u32) as u16;
    let rows = rows.clamp(MIN_ROWS as u32, MAX_ROWS as u32) as u16;
    (cols, rows)
}

/// A CSI sequence's parsed head: its first two parameters, whether it is a
/// private one (`ESC[?25h`, `ESC[>c`), and its final byte if the stream
/// holds it.
struct Csi {
    params: [u32; 2],
    private: bool,
    final_byte: Option<u8>,
}

/// Parses the CSI whose parameters start at `b[i]`; gives it and where the
/// bytes after it start.
fn csi(b: &[u8], mut i: usize) -> (Csi, usize) {
    let mut n = Csi {
        params: [0; 2],
        private: false,
        final_byte: None,
    };
    let mut k = 0;
    while i < b.len() {
        match b[i] {
            d @ b'0'..=b'9' => {
                if k < 2 {
                    n.params[k] = n.params[k]
                        .saturating_mul(10)
                        .saturating_add(u32::from(d - b'0'));
                }
            }
            b';' | b':' => k += 1,
            b'<'..=b'?' => n.private = true,
            0x20..=0x2f => {}
            f @ 0x40..=0x7e => {
                n.final_byte = Some(f);
                return (n, i + 1);
            }
            // Not a CSI after all: leave the byte to the main loop.
            _ => return (n, i),
        }
        i += 1;
    }
    (n, i)
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
    fn plain_text_sizes_the_screen_too() {
        // A full-width rule printed from column 0 with no moves past it
        // (review of #741): a screen narrower than it would wrap the rule
        // and scroll the bottom row away.
        let rule = "─".repeat(192);
        let status = "s".repeat(192);
        let s = format!("\x1b[H\r{rule}\x1b[50;1H{status}");
        let text = replay_text(s.as_bytes());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 50, "{text}");
        assert_eq!(lines[0], rule);
        assert_eq!(lines[49], status);
    }

    #[test]
    fn cursor_is_followed_through_relative_moves() {
        assert_eq!(geometry(b"\x1b[100;1H\x1b[A\x1b[3B"), (MIN_COLS, 102));
        assert_eq!(geometry(b"\x1b[1;300Hx"), (300, MIN_ROWS));
        // Private modes and strings move nothing.
        assert_eq!(
            geometry(b"\x1b[?2026h\x1b[>999C\x1b]0;\x1b[999C\x07"),
            (MIN_COLS, MIN_ROWS)
        );
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
