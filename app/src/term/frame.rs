//! rsz-term's frames (suzuri `crates/term/src/frame.rs`, decoded by
//! suzuri-view's `frame.js`) applied to a cell grid the view paints.
//!
//! A frame is a 32-byte header and the rows that changed. Each row names
//! its dirty columns `[x0, x1)`, which are cleared first, then the runs of
//! cells (same colours and attributes) that cover them; a trailing blank
//! run is left out. All little endian:
//!
//! - header: u8 'F', u8 flags (1 = full), u16 cols, u16 rows, u16 cursor x,
//!   u16 cursor y, u8 cursor shape, u8 version, u32 modes, u32 offset,
//!   u32 history, u32 seq, u16 nrows, u16 0
//! - row: u16 y, u16 nruns, u16 x0, u16 x1, then the runs
//! - run: u16 col, u16 n, u32 fg, u32 bg, u16 attrs, u16 0, n × u32 code
//!   points (0: the right half of a wide glyph)

/// The wire format this view decodes (header byte 11).
pub const FORMAT_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 32;
/// A run's bg meaning the terminal's own background (left unpainted).
pub const DEFAULT_BG: u32 = 0x0100_0000;

pub const A_BOLD: u16 = 1;
pub const A_ITALIC: u16 = 2;
pub const A_UNDERLINE: u16 = 4;
pub const A_STRIKE: u16 = 8;
pub const A_DIM: u16 = 16;
pub const A_HIDDEN: u16 = 32;
pub const A_SELECTED: u16 = 64;

pub const M_APP_CURSOR: u32 = 1;
pub const M_ALT_SCREEN: u32 = 4;
pub const M_FOCUS: u32 = 8;
pub const M_MOUSE_CLICK: u32 = 16;
pub const M_MOUSE_DRAG: u32 = 32;
pub const M_MOUSE_MOTION: u32 = 64;
pub const M_ALT_SCROLL: u32 = 256;
pub const M_KITTY_SHIFT: u32 = 10;
pub const MOUSE_MODES: u32 = M_MOUSE_CLICK | M_MOUSE_DRAG | M_MOUSE_MOTION;

/// A decoded header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub full: bool,
    pub cols: u16,
    pub rows: u16,
    pub cx: u16,
    pub cy: u16,
    /// 0 hidden, 1 block, 2 underline, 3 beam, 4 hollow.
    pub shape: u8,
    pub version: u8,
    pub modes: u32,
    /// Lines scrolled back from the end.
    pub offset: u32,
    pub history: u32,
    pub seq: u32,
    pub nrows: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub cp: u32,
    pub fg: u32,
    pub bg: u32,
    pub attrs: u16,
}

pub const BLANK: Cell = Cell {
    cp: b' ' as u32,
    fg: 0,
    bg: DEFAULT_BG,
    attrs: 0,
};

/// The terminal's screen as the last frames left it.
#[derive(Clone, Debug, Default)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
    /// Each row's version: bumped whenever a frame touches it (the view
    /// rebuilds only the rows that changed).
    pub ver: Vec<u64>,
    next_ver: u64,
    /// The last frame's header (cursor, modes, scroll offset, seq).
    pub head: Header,
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn u16(&mut self) -> Result<u16, String> {
        let s = self.b.get(self.p..self.p + 2).ok_or_else(short)?;
        self.p += 2;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.b.get(self.p..self.p + 4).ok_or_else(short)?;
        self.p += 4;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
}

fn short() -> String {
    "terminal frame cut short".to_string()
}

/// The header, or why the frame is refused (too short, another version).
pub fn header(b: &[u8]) -> Result<Header, String> {
    if b.len() < HEADER_LEN {
        return Err(format!(
            "terminal frame of {} bytes, shorter than its 32-byte header",
            b.len()
        ));
    }
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let h = Header {
        full: b[1] & 1 != 0,
        cols: u16_at(2),
        rows: u16_at(4),
        cx: u16_at(6),
        cy: u16_at(8),
        shape: b[10],
        version: b[11],
        modes: u32_at(12),
        offset: u32_at(16),
        history: u32_at(20),
        seq: u32_at(24),
        nrows: u16_at(28),
    };
    if h.version != FORMAT_VERSION {
        return Err(format!(
            "terminal format v{}, this app knows v{FORMAT_VERSION}",
            h.version
        ));
    }
    Ok(h)
}

impl Grid {
    pub fn cell(&self, col: u16, row: u16) -> Cell {
        if col >= self.cols || row >= self.rows {
            return BLANK;
        }
        self.cells[row as usize * self.cols as usize + col as usize]
    }

    pub fn row(&self, y: u16) -> &[Cell] {
        let w = self.cols as usize;
        let s = y as usize * w;
        &self.cells[s..s + w]
    }

    fn bump(&mut self, y: usize) {
        self.next_ver += 1;
        if let Some(v) = self.ver.get_mut(y) {
            *v = self.next_ver;
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        let mut cells = vec![BLANK; cols as usize * rows as usize];
        for y in 0..rows.min(self.rows) as usize {
            for x in 0..cols.min(self.cols) as usize {
                cells[y * cols as usize + x] = self.cells[y * self.cols as usize + x];
            }
        }
        self.cols = cols;
        self.rows = rows;
        self.cells = cells;
        self.ver = vec![0; rows as usize];
        for y in 0..rows as usize {
            self.bump(y);
        }
    }

    /// Applies one frame: the header always, then its dirty rows. A frame
    /// that is refused or cut short changes nothing past its header check.
    pub fn apply(&mut self, b: &[u8]) -> Result<Header, String> {
        let h = header(b)?;
        // Decode first, so a frame cut short leaves the grid as it was.
        let mut rows = Vec::with_capacity(h.nrows as usize);
        let mut r = Reader { b, p: HEADER_LEN };
        for _ in 0..h.nrows {
            let y = r.u16()?;
            let nruns = r.u16()?;
            let x0 = r.u16()?;
            let x1 = r.u16()?;
            let mut runs = Vec::with_capacity(nruns as usize);
            for _ in 0..nruns {
                let col = r.u16()?;
                let n = r.u16()?;
                let fg = r.u32()?;
                let bg = r.u32()?;
                let attrs = r.u16()?;
                r.u16()?;
                let mut cps = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    cps.push(r.u32()?);
                }
                runs.push((col, fg, bg, attrs, cps));
            }
            rows.push((y, x0, x1, runs));
        }
        self.resize(h.cols, h.rows);
        let w = self.cols as usize;
        for (y, x0, x1, runs) in rows {
            if y >= self.rows {
                continue;
            }
            let base = y as usize * w;
            for x in x0.min(self.cols)..x1.min(self.cols) {
                self.cells[base + x as usize] = BLANK;
            }
            for (col, fg, bg, attrs, cps) in runs {
                for (i, cp) in cps.into_iter().enumerate() {
                    let x = col as usize + i;
                    if x < w {
                        self.cells[base + x] = Cell { cp, fg, bg, attrs };
                    }
                }
            }
            self.bump(y as usize);
        }
        // The cursor's old and new rows repaint too (the view draws the
        // cursor with its row's glyph on top).
        let old = self.head;
        self.head = h;
        if old.cy != h.cy || old.cx != h.cx || old.shape != h.shape {
            self.bump(old.cy as usize);
            self.bump(h.cy as usize);
        }
        Ok(h)
    }

    /// The visible text, rows joined by newlines, trailing blanks trimmed.
    #[cfg(test)]
    pub fn text(&self) -> String {
        let mut out = Vec::new();
        for y in 0..self.rows {
            let s: String = self
                .row(y)
                .iter()
                .filter(|c| c.cp != 0)
                .map(|c| char::from_u32(c.cp).unwrap_or(' '))
                .collect();
            out.push(s.trim_end().to_string());
        }
        out.join("\n")
    }
}

/// Encodes a frame (rsz-term's encoder, for the demo scene and tests).
pub struct Encoder {
    out: Vec<u8>,
    nrows: u16,
}

impl Encoder {
    pub fn new(h: Header) -> Encoder {
        let mut out = Vec::with_capacity(4096);
        out.push(b'F');
        out.push(h.full as u8);
        for v in [h.cols, h.rows, h.cx, h.cy] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(h.shape);
        out.push(FORMAT_VERSION);
        for v in [h.modes, h.offset, h.history, h.seq] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0, 0, 0]);
        Encoder { out, nrows: 0 }
    }

    /// One row: its cells are columns `[x0, x0 + cells.len())`.
    pub fn row(&mut self, y: u16, x0: u16, cells: &[Cell]) {
        let o = &mut self.out;
        o.extend_from_slice(&y.to_le_bytes());
        let at = o.len();
        o.extend_from_slice(&0u16.to_le_bytes());
        o.extend_from_slice(&x0.to_le_bytes());
        o.extend_from_slice(&(x0 + cells.len() as u16).to_le_bytes());
        let mut nruns = 0u16;
        let mut i = 0;
        while i < cells.len() {
            let c = cells[i];
            let mut j = i + 1;
            while j < cells.len()
                && cells[j].fg == c.fg
                && cells[j].bg == c.bg
                && cells[j].attrs == c.attrs
            {
                j += 1;
            }
            let blank_tail = j == cells.len()
                && c.bg == DEFAULT_BG
                && c.attrs & (A_UNDERLINE | A_STRIKE | A_SELECTED) == 0
                && cells[i..j].iter().all(|c| c.cp == b' ' as u32);
            if !blank_tail {
                o.extend_from_slice(&(x0 + i as u16).to_le_bytes());
                o.extend_from_slice(&((j - i) as u16).to_le_bytes());
                o.extend_from_slice(&c.fg.to_le_bytes());
                o.extend_from_slice(&c.bg.to_le_bytes());
                o.extend_from_slice(&c.attrs.to_le_bytes());
                o.extend_from_slice(&0u16.to_le_bytes());
                for cell in &cells[i..j] {
                    o.extend_from_slice(&cell.cp.to_le_bytes());
                }
                nruns += 1;
            }
            i = j;
        }
        o[at..at + 2].copy_from_slice(&nruns.to_le_bytes());
        self.nrows += 1;
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.out[28..30].copy_from_slice(&self.nrows.to_le_bytes());
        self.out
    }
}

/// Cells for `s` in one style (the demo's and the tests' rows).
pub fn cells(s: &str, fg: u32, bg: u32, attrs: u16) -> Vec<Cell> {
    s.chars()
        .map(|ch| Cell {
            cp: ch as u32,
            fg,
            bg,
            attrs,
        })
        .collect()
}

/// A canned full frame for the demo scene: a prompt, coloured `ls`
/// output, a selection, bold/italic/underline/dim/strike, a box drawn
/// with box glyphs, blocks and shades, a wide glyph, and the cursor.
pub fn demo_frame(cols: u16, rows: u16) -> Vec<u8> {
    const FG: u32 = 0xd0d0d8;
    const GREEN: u32 = 0x5fd787;
    const BLUE: u32 = 0x5f87ff;
    const VIOLET: u32 = 0xb8a0c8;
    const AMBER: u32 = 0xe0a526;
    const RED: u32 = 0xe5534b;
    const MUTED: u32 = 0x8b8f98;
    let d = DEFAULT_BG;
    let mut lines: Vec<Vec<Cell>> = Vec::new();
    let prompt = |cmd: &str| {
        let mut v = cells("stephen", GREEN, d, A_BOLD);
        v.extend(cells(" ~/projects/irori ", BLUE, d, 0));
        v.extend(cells("main", VIOLET, d, 0));
        v.extend(cells(" % ", FG, d, 0));
        v.extend(cells(cmd, FG, d, 0));
        v
    };
    lines.push(prompt("ls"));
    let mut ls = cells("Cargo.toml  ", FG, d, 0);
    ls.extend(cells("app", BLUE, d, A_BOLD));
    ls.extend(cells("  ", FG, d, 0));
    ls.extend(cells("crates", BLUE, d, A_BOLD));
    ls.extend(cells("  ", FG, d, 0));
    ls.extend(cells("docs", BLUE, d, A_BOLD));
    ls.extend(cells("  README.md  ", FG, d, 0));
    ls.extend(cells("scripts", BLUE, d, A_BOLD));
    lines.push(ls);
    lines.push(prompt("git status --short"));
    let mut st = cells(" M", RED, d, 0);
    st.extend(cells(" app/src/term/mod.rs", FG, d, 0));
    lines.push(st);
    let mut st = cells("??", RED, d, 0);
    st.extend(cells(" app/src/term/frame.rs", FG, d, 0));
    lines.push(st);
    let mut styles = cells("bold", FG, d, A_BOLD);
    styles.extend(cells(" ", FG, d, 0));
    styles.extend(cells("italic", FG, d, A_ITALIC));
    styles.extend(cells(" ", FG, d, 0));
    styles.extend(cells("underline", FG, d, A_UNDERLINE));
    styles.extend(cells(" ", FG, d, 0));
    styles.extend(cells("dim", FG, d, A_DIM));
    styles.extend(cells(" ", FG, d, 0));
    styles.extend(cells("strike", FG, d, A_STRIKE));
    styles.extend(cells(" ", FG, d, 0));
    styles.extend(cells(" on amber ", 0x101010, AMBER, 0));
    styles.extend(cells(" ", FG, d, 0));
    styles.extend(cells("selected text", FG, d, A_SELECTED));
    lines.push(styles);
    // A wide glyph takes two cells: its code point, then 0.
    let mut wide = cells("wide: ", FG, d, 0);
    for ch in ['日', '本'] {
        wide.push(Cell {
            cp: ch as u32,
            fg: AMBER,
            bg: d,
            attrs: 0,
        });
        wide.push(Cell {
            cp: 0,
            fg: AMBER,
            bg: d,
            attrs: 0,
        });
    }
    wide.extend(cells("  arrows: → ← ↑ ↓  ✓ done", FG, d, 0));
    lines.push(wide);
    lines.push(Vec::new());
    lines.push(cells("╭──────────────────────────────╮", VIOLET, d, 0));
    lines.push(cells("│  irori: a terminal in Rune   │", VIOLET, d, 0));
    lines.push(cells("├──────────────┬───────────────┤", VIOLET, d, 0));
    lines.push(cells("│ ┏━━━━━━━━┓   │ ╔═══════╗     │", VIOLET, d, 0));
    lines.push(cells("│ ┃ heavy  ┃   │ ║ double║     │", VIOLET, d, 0));
    lines.push(cells("│ ┗━━━━━━━━┛   │ ╚═══════╝     │", VIOLET, d, 0));
    lines.push(cells("╰──────────────┴───────────────╯", VIOLET, d, 0));
    lines.push(cells("blocks ░░▒▒▓▓██ ▀▄▌▐ ▁▂▃▄▅▆▇█", GREEN, d, 0));
    lines.push(Vec::new());
    lines.push(cells("$ echo done", MUTED, d, 0));
    let last = prompt("");
    let cx = last.len() as u16;
    let cy = lines.len() as u16;
    lines.push(last);
    let mut e = Encoder::new(Header {
        full: true,
        cols,
        rows,
        cx,
        cy,
        shape: 1,
        version: FORMAT_VERSION,
        ..Header::default()
    });
    for y in 0..rows {
        let mut row = lines.get(y as usize).cloned().unwrap_or_default();
        row.truncate(cols as usize);
        while row.len() < cols as usize {
            row.push(BLANK);
        }
        e.row(y, 0, &row);
    }
    e.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(cols: u16, rows: u16, full: bool) -> Header {
        Header {
            full,
            cols,
            rows,
            shape: 1,
            version: FORMAT_VERSION,
            ..Header::default()
        }
    }

    #[test]
    fn header_is_checked() {
        assert!(header(&[0u8; 10]).unwrap_err().contains("10 bytes"));
        let mut f = Encoder::new(head(4, 1, true)).finish();
        assert!(header(&f).is_ok());
        f[11] = 9;
        assert_eq!(header(&f).unwrap_err(), "terminal format v9, this app knows v1");
    }

    #[test]
    fn full_frame_then_dirty_range() {
        let mut g = Grid::default();
        let mut e = Encoder::new(head(6, 2, true));
        e.row(0, 0, &cells("hello ", 1, DEFAULT_BG, 0));
        e.row(1, 0, &cells("world!", 2, DEFAULT_BG, A_BOLD));
        g.apply(&e.finish()).unwrap();
        assert_eq!(g.text(), "hello\nworld!");
        assert_eq!(g.cell(0, 1).attrs, A_BOLD);
        let v0 = g.ver[0];
        // Columns 1..3 of row 0 change; the rest of the row stays.
        let mut e = Encoder::new(head(6, 2, false));
        e.row(0, 1, &cells("EL", 3, DEFAULT_BG, 0));
        g.apply(&e.finish()).unwrap();
        assert_eq!(g.text(), "hELlo\nworld!");
        assert!(g.ver[0] > v0);
    }

    #[test]
    fn dirty_range_clears_and_blank_tail_is_left_out() {
        let mut g = Grid::default();
        let mut e = Encoder::new(head(5, 1, true));
        e.row(0, 0, &cells("abcde", 1, DEFAULT_BG, 0));
        g.apply(&e.finish()).unwrap();
        let mut e = Encoder::new(head(5, 1, false));
        e.row(0, 0, &cells("x    ", 1, DEFAULT_BG, 0));
        let f = e.finish();
        // One run: the blank tail isn't sent, the clear covers it.
        assert_eq!(u16::from_le_bytes([f[34], f[35]]), 1);
        g.apply(&f).unwrap();
        assert_eq!(g.text(), "x");
    }

    #[test]
    fn short_frame_changes_nothing() {
        let mut g = Grid::default();
        let mut e = Encoder::new(head(3, 1, true));
        e.row(0, 0, &cells("abc", 1, DEFAULT_BG, 0));
        g.apply(&e.finish()).unwrap();
        let mut e = Encoder::new(head(3, 1, false));
        e.row(0, 0, &cells("xyz", 1, DEFAULT_BG, 0));
        let mut f = e.finish();
        f.truncate(f.len() - 3);
        assert!(g.apply(&f).is_err());
        assert_eq!(g.text(), "abc");
    }

    #[test]
    fn resize_keeps_what_fits() {
        let mut g = Grid::default();
        let mut e = Encoder::new(head(4, 2, true));
        e.row(0, 0, &cells("abcd", 1, DEFAULT_BG, 0));
        e.row(1, 0, &cells("efgh", 1, DEFAULT_BG, 0));
        g.apply(&e.finish()).unwrap();
        g.apply(&Encoder::new(head(2, 3, false)).finish()).unwrap();
        assert_eq!((g.cols, g.rows), (2, 3));
        assert_eq!(g.text(), "ab\nef\n");
    }

    #[test]
    fn wide_glyph_keeps_its_right_half() {
        let mut g = Grid::default();
        let f = demo_frame(60, 24);
        let h = g.apply(&f).unwrap();
        assert_eq!((h.cols, h.rows), (60, 24));
        let row = g.row(6);
        let at = row.iter().position(|c| c.cp == '日' as u32).unwrap();
        assert_eq!(row[at + 1].cp, 0);
        assert!(g.text().contains("irori: a terminal in Rune"));
    }
}
