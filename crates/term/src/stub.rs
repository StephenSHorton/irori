//! The PTY half on Android and iOS: the same API, and no shells (design
//! section 2). A phone watches a desktop's terminal instead (S6).

use std::sync::Arc;

use crate::{Link, Mouse, NOT_HERE, OpenSpec, Scope, SelectKind, TermId, TermInfo, Viewer};

#[derive(Default)]
pub struct Terms;

impl Terms {
    pub fn new() -> Terms {
        Terms
    }
    pub fn supported(&self) -> bool {
        false
    }
    pub fn open(&self, _spec: OpenSpec) -> Result<TermInfo, String> {
        Err(NOT_HERE.into())
    }
    pub fn attach(&self, _id: TermId, _viewer: Arc<Viewer>) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn detach(&self, _id: TermId) {}
    pub fn write(&self, _id: TermId, _data: Vec<u8>) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn paste(&self, _id: TermId, _text: &str) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn resize(&self, _id: TermId, _cols: u16, _rows: u16) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn scroll(&self, _id: TermId, _delta: i32) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn cell_px(&self, _id: TermId, _w: u16, _h: u16) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn focus(&self, _id: TermId, _on: bool) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn redraw(&self, _id: TermId) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn ack(&self, _id: TermId, _seq: u32) -> Result<(), String> {
        Ok(())
    }
    pub fn screen_text(&self, _id: TermId) -> Result<String, String> {
        Err(NOT_HERE.into())
    }
    pub fn select_start(&self, _id: TermId, _col: u16, _row: u16, _right: bool, _kind: SelectKind) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn select_extend(&self, _id: TermId, _col: u16, _row: u16, _right: bool) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn select_clear(&self, _id: TermId) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn selection_text(&self, _id: TermId) -> Result<Option<String>, String> {
        Err(NOT_HERE.into())
    }
    pub fn link_at(&self, _id: TermId, _col: u16, _row: u16) -> Result<Option<Link>, String> {
        Err(NOT_HERE.into())
    }
    pub fn mouse(&self, _id: TermId, _m: Mouse) -> Result<bool, String> {
        Err(NOT_HERE.into())
    }
    pub fn close(&self, _id: TermId) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    pub fn in_scope(&self, _scope: &Scope) -> Vec<TermInfo> {
        Vec::new()
    }
    pub fn list(&self) -> Vec<(TermId, Scope, bool)> {
        Vec::new()
    }
    pub fn close_all(&self) {}
}
