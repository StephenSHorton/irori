//! Loading shapes (the React app's common/loading-shapes.tsx and the
//! `Loading` that measured them into skeletons, card #598): blocks in the
//! size and rhythm of what loads, pulsing softly. Each text line of the
//! React shapes is a bar here, as the skeleton drew it.

use rune::{El, LoopSpec, col, el, row};

use crate::ui::Frame;

/// One skeleton block, `w` wide (`None`: the full width), `h` tall.
pub fn block(f: &Frame, key: &str, w: Option<f32>, h: f32, radius: f32) -> El {
    let b = el()
        .key(key.to_string())
        .h(h)
        .radius(radius)
        .fill(f.k.c.layer_03)
        .looping(LoopSpec::opacity(1.6, 0.0, 0.45, 1.0))
        .a11y_hidden();
    match w {
        Some(w) => b.w(w),
        None => b.w_full(),
    }
}

/// A text line of `chars` characters at a font of `size`: a bar of about
/// that length.
fn bar(f: &Frame, key: &str, chars: usize, size: f32) -> El {
    block(
        f,
        key,
        Some((chars as f32 * size * 0.5).min(560.0)),
        size * 0.8,
        3.0,
    )
}

fn panel(f: &Frame, key: &str) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .min_w(0.0)
        .radius(10.0)
        .fill(f.k.c.layer_01)
        .border(1.0, f.k.c.border_muted)
}

/// A run of thread messages: the time column, a name, a line or two.
pub fn message_rows_shape(f: &Frame, key: &str, rows: usize) -> El {
    let lines = [(12, 55), (7, 40), (12, 100), (12, 22)];
    let mut c = col().key(key.to_string()).w_full().gap(12.0);
    for i in 0..rows {
        let (who, t) = lines[i % lines.len()];
        c = c.child(
            row()
                .key(format!("{key}/{i}"))
                .w_full()
                .gap(8.0)
                .child(el().w(56.0).child(bar(f, &format!("{key}/{i}/t"), 5, 12.5)))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(bar(f, &format!("{key}/{i}/w"), who, 12.5))
                        .child(bar(f, &format!("{key}/{i}/x"), t, 12.5)),
                ),
        );
    }
    c
}

/// A column of list rows: a dot, a title, a short line under it.
pub fn list_rows_shape(f: &Frame, key: &str, rows: usize, sub: bool) -> El {
    let titles = [30, 26, 14, 34, 17];
    let mut c = col().key(key.to_string()).w_full().gap(4.0);
    for i in 0..rows {
        let mut t = col().gap(4.0).min_w(0.0).child(bar(
            f,
            &format!("{key}/{i}/t"),
            titles[i % titles.len()],
            13.0,
        ));
        if sub {
            t = t.child(bar(f, &format!("{key}/{i}/s"), 21, 11.5));
        }
        c = c.child(
            row()
                .key(format!("{key}/{i}"))
                .px(8.0)
                .py(6.0)
                .gap(8.0)
                .child(block(f, &format!("{key}/{i}/d"), Some(16.0), 16.0, 8.0))
                .child(t),
        );
    }
    c
}

/// A run of Talk posts: a face, the name and time, the message.
pub fn post_rows_shape(f: &Frame, key: &str, rows: usize) -> El {
    let posts = [(12, 55), (3, 40), (12, 100), (12, 22)];
    let mut c = col().key(key.to_string()).w_full().gap(2.0);
    for i in 0..rows {
        let (who, t) = posts[i % posts.len()];
        c = c.child(
            row()
                .key(format!("{key}/{i}"))
                .w_full()
                .px(8.0)
                .py(6.0)
                .gap(10.0)
                .child(block(f, &format!("{key}/{i}/f"), Some(26.0), 26.0, 0.0))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(
                            row()
                                .gap(8.0)
                                .child(bar(f, &format!("{key}/{i}/w"), who, 13.0))
                                .child(bar(f, &format!("{key}/{i}/t"), 5, 11.0)),
                        )
                        .child(bar(f, &format!("{key}/{i}/x"), t, 13.0)),
                ),
        );
    }
    c
}

/// One card tile: its chips, title, progress bar, the line under it.
pub fn card_tile_shape(f: &Frame, key: &str) -> El {
    panel(f, key)
        .padding(14.0)
        .gap(10.0)
        .child(
            row()
                .gap(6.0)
                .child(bar(f, &format!("{key}/r"), 6, 12.0))
                .child(block(f, &format!("{key}/c0"), Some(80.0), 20.0, 6.0))
                .child(block(f, &format!("{key}/c1"), Some(40.0), 20.0, 6.0)),
        )
        .child(bar(f, &format!("{key}/t"), 34, 14.0))
        .child(block(f, &format!("{key}/p"), None, 4.0, 0.0))
        .child(bar(f, &format!("{key}/l"), 26, 11.0))
}

/// A grid of card tiles in the masonry's 260-point columns.
pub fn tile_grid_shape(f: &Frame, key: &str, tiles: usize) -> El {
    let mut r = row().key(key.to_string()).w_full().gap(12.0).flex_wrap();
    for i in 0..tiles {
        r = r.child(
            el().key(format!("{key}/{i}"))
                .w(260.0)
                .grow(1.0)
                .child(card_tile_shape(f, &format!("{key}/{i}/t"))),
        );
    }
    r
}

/// One Needs you row: kind, where, when, the title, a line and buttons.
pub fn need_row_shape(f: &Frame, key: &str) -> El {
    panel(f, key)
        .px(16.0)
        .py(12.0)
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(block(f, &format!("{key}/k"), Some(80.0), 20.0, 6.0))
                .child(bar(f, &format!("{key}/r"), 4, 12.5))
                .child(el().grow(1.0))
                .child(bar(f, &format!("{key}/w"), 6, 12.5)),
        )
        .child(bar(f, &format!("{key}/t"), 36, 15.0))
        .child(block(f, &format!("{key}/l0"), None, 11.0, 3.0))
        .child(bar(f, &format!("{key}/l1"), 60, 14.0))
        .child(
            row()
                .gap(6.0)
                .child(block(f, &format!("{key}/b0"), Some(64.0), 32.0, 6.0))
                .child(block(f, &format!("{key}/b1"), Some(96.0), 32.0, 6.0)),
        )
}

/// Home's Needs you: its heading and a few rows.
pub fn needs_you_shape(f: &Frame, key: &str, rows: usize) -> El {
    let mut c =
        col()
            .key(key.to_string())
            .w_full()
            .gap(10.0)
            .child(bar(f, &format!("{key}/h"), 12, 16.0));
    for i in 0..rows {
        c = c.child(need_row_shape(f, &format!("{key}/{i}")));
    }
    c
}
