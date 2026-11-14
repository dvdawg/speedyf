//! Open project text files: one in-memory copy per file, however many views
//! edit it (projects.md sec. 13, "One owner, many views").
//!
//! A view (the reader's drawer, the Projects screen's Notepad view, in any
//! window) is a client. It sends an edit as one range replacement against
//! the revision it last saw; the core moves the edit past whatever landed
//! after that revision, applies it, bumps the revision and broadcasts the
//! edit as applied to every window - the sender included, which is how the
//! sender learns where its edit fell in the order. Offsets are UTF-16 code
//! units, the unit the webview's strings are measured in.
//!
//! Saving is the core's too: about half a second after the last edit
//! (`SAVE_AFTER`), atomically. Once a second the files of open documents
//! are compared with what was last read or written: a change from outside
//! reloads a clean document and is kept beside a dirty one.

use super::projectfiles;
use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Quiet time after the last edit before a document is written.
pub const SAVE_AFTER: Duration = Duration::from_millis(500);
/// Edits kept for moving a late edit past: far more than two people typing
/// at once can put between a send and its arrival.
const LOG_LIMIT: usize = 2_000;
/// Largest open text document.
pub const MAX_DOC_BYTES: usize = projectfiles::MAX_FILE_BYTES;

/// Which file: an extension's, for a project.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocKey {
    pub extension_id: String,
    pub project_id: String,
    pub file: String,
}

impl DocKey {
    pub fn path(&self, data_dir: &Path) -> AppResult<PathBuf> {
        projectfiles::file_path(data_dir, &self.extension_id, &self.project_id, &self.file)
    }
}

/// Replace UTF-16 units `[from, to)` with `insert`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub from: usize,
    pub to: usize,
    pub insert: String,
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The byte offset of UTF-16 offset `at`, clamped to the text and to a
/// character boundary (an offset inside a surrogate pair moves past it).
fn byte_at(text: &str, at: usize) -> usize {
    // ASCII text, the usual case: one unit is one byte.
    if text.is_ascii() {
        return at.min(text.len());
    }
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= at {
            return index;
        }
        units += ch.len_utf16();
    }
    text.len()
}

impl Op {
    /// Apply to `text`, clamping a range that runs past its end. (An open
    /// document applies edits in place; this is the same rule on a copy.)
    #[cfg(test)]
    pub fn apply(&self, text: &str) -> String {
        let from = byte_at(text, self.from);
        let to = byte_at(text, self.to.max(self.from)).max(from);
        let mut out = String::with_capacity(text.len() + self.insert.len());
        out.push_str(&text[..from]);
        out.push_str(&self.insert);
        out.push_str(&text[to..]);
        out
    }

    /// Where position `pos` lands once `self` is applied. A position inside
    /// the replaced range moves to the end of the inserted text; one exactly
    /// at an insertion point stays before it unless `after` (the position
    /// belongs to an edit ordered after this one).
    pub fn map(&self, pos: usize, after: bool) -> usize {
        let inserted = utf16_len(&self.insert);
        if self.from == self.to {
            if pos < self.from || (pos == self.from && !after) {
                pos
            } else {
                pos + inserted
            }
        } else if pos <= self.from {
            pos
        } else if pos >= self.to {
            pos + inserted - (self.to - self.from)
        } else {
            self.from + inserted
        }
    }

    /// This edit, made against the same text as `earlier`, moved to apply
    /// after `earlier` has been. `later` says which of the two the order
    /// puts second, which decides a tie between two insertions at one
    /// position. Overlapping edits keep both insertions; the overlap of
    /// their deletions is deleted once.
    pub fn transform(&self, earlier: &Op, later: bool) -> Op {
        let from = earlier.map(self.from, later);
        let to = earlier.map(self.to, later).max(from);
        Op {
            from,
            to,
            insert: self.insert.clone(),
        }
    }
}

/// Size and modification time of a file as last read or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DiskStamp {
    len: u64,
    modified: Option<SystemTime>,
}

fn stamp_of(path: &Path) -> Option<DiskStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(DiskStamp {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

struct OpenDoc {
    text: String,
    /// Its length in UTF-16 units, kept rather than counted per edit.
    len16: usize,
    /// Whether it is all ASCII, where a unit is a byte.
    ascii: bool,
    rev: u64,
    /// (revision it produced, edit as applied), oldest first.
    log: VecDeque<(u64, Op)>,
    dirty: bool,
    last_edit: Instant,
    disk: Option<DiskStamp>,
    /// Views holding it open, across windows.
    users: usize,
}

impl OpenDoc {
    fn set_text(&mut self, text: String) {
        self.len16 = utf16_len(&text);
        self.ascii = text.is_ascii();
        self.text = text;
    }
}

/// What a broadcast tells the windows.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DocEvent {
    /// An edit, as applied, producing `rev`. `origin` is the client that
    /// sent it; `len` the document's UTF-16 length after it, for a client
    /// to check itself against.
    Edit {
        key: DocKey,
        rev: u64,
        op: Op,
        origin: String,
        len: usize,
    },
    /// The whole text replaced from outside the views (the file changed on
    /// disk, a backup was restored). A client drops what it had.
    Reset { key: DocKey, rev: u64, text: String },
    /// The document is gone: its project was deleted, or the data folder
    /// moved. Views close and say why.
    Closed { key: DocKey, reason: String },
    /// Something the views should say once ("Changed on disk; ...").
    Notice { key: DocKey, message: String },
}

pub type Emit = Box<dyn Fn(DocEvent) + Send + Sync>;

/// Every open project text document.
pub struct ProjectDocs {
    docs: HashMap<DocKey, OpenDoc>,
    emit: Option<Emit>,
}

impl Default for ProjectDocs {
    fn default() -> Self {
        Self::new()
    }
}

/// What `open` answers: the text and the revision it is at.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedDoc {
    pub text: String,
    pub rev: u64,
}

impl ProjectDocs {
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
            emit: None,
        }
    }

    pub fn set_emitter(&mut self, emit: Emit) {
        self.emit = Some(emit);
    }

    fn send(&self, event: DocEvent) {
        if let Some(emit) = &self.emit {
            emit(event);
        }
    }

    /// Open (or join) a document. A missing file opens empty.
    pub fn open(&mut self, data_dir: &Path, key: &DocKey) -> AppResult<OpenedDoc> {
        if let Some(doc) = self.docs.get_mut(key) {
            doc.users += 1;
            return Ok(OpenedDoc {
                text: doc.text.clone(),
                rev: doc.rev,
            });
        }
        let path = key.path(data_dir)?;
        let text = match projectfiles::read(&path)? {
            Some(bytes) => String::from_utf8(bytes)
                .map_err(|_| AppError::Malformed(format!("{} is not UTF-8 text", key.file)))?,
            None => String::new(),
        };
        let doc = OpenDoc {
            len16: utf16_len(&text),
            ascii: text.is_ascii(),
            text: text.clone(),
            rev: 1,
            log: VecDeque::new(),
            dirty: false,
            last_edit: Instant::now(),
            disk: stamp_of(&path),
            users: 1,
        };
        self.docs.insert(key.clone(), doc);
        Ok(OpenedDoc { text, rev: 1 })
    }

    /// A view let go. The last one out leaves a clean document unloaded; a
    /// dirty one stays until it is saved.
    pub fn close(&mut self, key: &DocKey) {
        if let Some(doc) = self.docs.get_mut(key) {
            doc.users = doc.users.saturating_sub(1);
            if doc.users == 0 && !doc.dirty {
                self.docs.remove(key);
            }
        }
    }

    pub fn is_open(&self, key: &DocKey) -> bool {
        self.docs.contains_key(key)
    }

    /// The live text of an open document.
    pub fn text(&self, key: &DocKey) -> Option<String> {
        self.docs.get(key).map(|doc| doc.text.clone())
    }

    /// Apply a view's edit, made against `base_rev`. Returns the revision
    /// it produced and the edit as applied, which is also broadcast.
    pub fn edit(
        &mut self,
        key: &DocKey,
        base_rev: u64,
        op: Op,
        origin: &str,
    ) -> AppResult<(u64, Op)> {
        let doc = self
            .docs
            .get_mut(key)
            .ok_or_else(|| AppError::Unsupported("that document is not open".into()))?;
        if base_rev > doc.rev {
            return Err(AppError::Malformed("an edit from the future".into()));
        }
        let oldest = doc.log.front().map(|(rev, _)| *rev).unwrap_or(doc.rev + 1);
        if base_rev < doc.rev && base_rev + 1 < oldest {
            return Err(AppError::Unsupported(
                "the edit is too old to place; reopen the document".into(),
            ));
        }
        let mut op = op;
        for (rev, earlier) in &doc.log {
            if *rev > base_rev {
                op = op.transform(earlier, true);
            }
        }
        // Clamped the way `apply` clamps it, so what is broadcast replays.
        let len = doc.len16;
        op.from = op.from.min(len);
        op.to = op.to.clamp(op.from, len);
        let (from, to) = if doc.ascii {
            (op.from, op.to)
        } else {
            (byte_at(&doc.text, op.from), byte_at(&doc.text, op.to))
        };
        if doc.text.len() - (to - from) + op.insert.len() > MAX_DOC_BYTES {
            return Err(AppError::Unsupported(format!(
                "a project file is limited to {} MB",
                MAX_DOC_BYTES / (1024 * 1024)
            )));
        }
        doc.text.replace_range(from..to, &op.insert);
        doc.len16 = doc.len16 - (op.to - op.from) + utf16_len(&op.insert);
        doc.ascii = doc.ascii && op.insert.is_ascii();
        doc.rev += 1;
        doc.dirty = true;
        doc.last_edit = Instant::now();
        doc.log.push_back((doc.rev, op.clone()));
        while doc.log.len() > LOG_LIMIT {
            doc.log.pop_front();
        }
        let (rev, len) = (doc.rev, doc.len16);
        self.send(DocEvent::Edit {
            key: key.clone(),
            rev,
            op: op.clone(),
            origin: origin.to_owned(),
            len,
        });
        Ok((rev, op))
    }

    /// Replace an open document's whole text from outside the views (an
    /// extension's `projectFiles.write`). Broadcast as an edit, so views
    /// keep their place and their unsent typing.
    pub fn replace(&mut self, key: &DocKey, text: &str, origin: &str) -> AppResult<(u64, Op)> {
        let len = self
            .docs
            .get(key)
            .map(|doc| utf16_len(&doc.text))
            .ok_or_else(|| AppError::Unsupported("that document is not open".into()))?;
        let rev = self.docs[key].rev;
        self.edit(
            key,
            rev,
            Op {
                from: 0,
                to: len,
                insert: text.to_owned(),
            },
            origin,
        )
    }

    /// Write what is due: documents edited and then quiet for `SAVE_AFTER`
    /// (all dirty ones when `force`). Returns the keys written.
    pub fn save_due(&mut self, data_dir: &Path, now: Instant, force: bool) -> Vec<DocKey> {
        let mut saved = Vec::new();
        let mut unload = Vec::new();
        for (key, doc) in self.docs.iter_mut() {
            if !doc.dirty || (!force && now.saturating_duration_since(doc.last_edit) < SAVE_AFTER) {
                continue;
            }
            let Ok(path) = key.path(data_dir) else {
                continue;
            };
            match projectfiles::write_atomic(&path, doc.text.as_bytes()) {
                Ok(()) => {
                    doc.dirty = false;
                    doc.disk = stamp_of(&path);
                    saved.push(key.clone());
                    if doc.users == 0 {
                        unload.push(key.clone());
                    }
                }
                Err(error) => log::warn!("cannot save {}: {error}", path.display()),
            }
        }
        for key in unload {
            self.docs.remove(&key);
        }
        saved
    }

    /// Compare open documents with their files. A file that changed from
    /// outside reloads a clean document (`Reset`); beside a dirty one, the
    /// outside version is kept as `<name>.external-<time>.<ext>` and the
    /// views are told (`Notice`), and the next save writes the views' text.
    pub fn check_disk(&mut self, data_dir: &Path, stamp: &str) {
        let mut events = Vec::new();
        for (key, doc) in self.docs.iter_mut() {
            let Ok(path) = key.path(data_dir) else {
                continue;
            };
            let now = stamp_of(&path);
            if now == doc.disk {
                continue;
            }
            doc.disk = now;
            let outside = match projectfiles::read(&path) {
                Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
                Ok(None) => String::new(),
                Err(_) => continue,
            };
            if outside == doc.text {
                continue;
            }
            if doc.dirty {
                let kept = external_name(&key.file, stamp);
                if let Ok(kept_path) =
                    projectfiles::file_path(data_dir, &key.extension_id, &key.project_id, &kept)
                {
                    if projectfiles::write_atomic(&kept_path, outside.as_bytes()).is_ok() {
                        events.push(DocEvent::Notice {
                            key: key.clone(),
                            message: format!(
                                "Changed on disk while you were editing; your version is kept and the other was saved as {kept}"
                            ),
                        });
                    }
                }
            } else {
                doc.set_text(outside.clone());
                doc.rev += 1;
                // Nothing in the log can be moved past a reset.
                doc.log.clear();
                events.push(DocEvent::Reset {
                    key: key.clone(),
                    rev: doc.rev,
                    text: outside,
                });
            }
        }
        for event in events {
            self.send(event);
        }
    }

    /// Save everything dirty now, then drop every document whose key
    /// `matches`, telling its views why. For a project being deleted, and
    /// for the data folder moving away.
    pub fn close_where(
        &mut self,
        data_dir: &Path,
        matches: impl Fn(&DocKey) -> bool,
        reason: &str,
    ) {
        self.save_due(data_dir, Instant::now(), true);
        let keys: Vec<DocKey> = self.docs.keys().filter(|k| matches(k)).cloned().collect();
        for key in keys {
            self.docs.remove(&key);
            self.send(DocEvent::Closed {
                key,
                reason: reason.to_owned(),
            });
        }
    }

    /// Re-read every open document from disk (a backup was restored under
    /// it) and tell the views.
    pub fn reload_all(&mut self, data_dir: &Path) {
        let mut events = Vec::new();
        for (key, doc) in self.docs.iter_mut() {
            let Ok(path) = key.path(data_dir) else {
                continue;
            };
            let text = projectfiles::read(&path)
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default();
            doc.set_text(text.clone());
            doc.rev += 1;
            doc.log.clear();
            doc.dirty = false;
            doc.disk = stamp_of(&path);
            events.push(DocEvent::Reset {
                key: key.clone(),
                rev: doc.rev,
                text,
            });
        }
        for event in events {
            self.send(event);
        }
    }
}

/// `notepad.md` -> `notepad.external-20261113-101500.md`.
fn external_name(file: &str, stamp: &str) -> String {
    let (dir, name) = match file.rsplit_once('/') {
        Some((dir, name)) => (format!("{dir}/"), name),
        None => (String::new(), file),
    };
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{dir}{stem}.external-{stamp}.{ext}"),
        _ => format!("{dir}{name}.external-{stamp}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn key() -> DocKey {
        DocKey {
            extension_id: "dev.speedyf.notepad".into(),
            project_id: "p1".into(),
            file: "notepad.md".into(),
        }
    }

    fn op(from: usize, to: usize, insert: &str) -> Op {
        Op {
            from,
            to,
            insert: insert.into(),
        }
    }

    fn recording() -> (ProjectDocs, Arc<Mutex<Vec<DocEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let mut docs = ProjectDocs::new();
        docs.set_emitter(Box::new(move |event| sink.lock().unwrap().push(event)));
        (docs, events)
    }

    #[test]
    fn utf16_offsets_survive_non_ascii_text() {
        // An astral character is two UTF-16 units.
        let text = "a\u{1d49c}b\u{e9}";
        assert_eq!(op(3, 4, "B").apply(text), "a\u{1d49c}B\u{e9}");
        assert_eq!(op(5, 5, "!").apply(text), "a\u{1d49c}b\u{e9}!");
        assert_eq!(op(99, 120, "?").apply(text), "a\u{1d49c}b\u{e9}?");
    }

    #[test]
    fn two_windows_typing_at_once_converge() {
        let dir = tempfile::tempdir().unwrap();
        let (mut docs, events) = recording();
        let k = key();
        // Two windows open the same note.
        let a = docs.open(dir.path(), &k).unwrap();
        let b = docs.open(dir.path(), &k).unwrap();
        assert_eq!(a, b);
        docs.edit(&k, a.rev, op(0, 0, "hello world"), "a").unwrap();
        // Both now at rev 2. A types at the end, B at the start, both
        // against rev 2.
        docs.edit(&k, 2, op(11, 11, "!"), "a").unwrap();
        docs.edit(&k, 2, op(0, 5, "HELLO"), "b").unwrap();
        assert_eq!(docs.text(&k).unwrap(), "HELLO world!");
        // Every edit was broadcast with its place in the order, so a window
        // replaying them in order gets the same text.
        let mut replay = String::new();
        for event in events.lock().unwrap().iter() {
            if let DocEvent::Edit { op, len, .. } = event {
                replay = op.apply(&replay);
                assert_eq!(replay.encode_utf16().count(), *len);
            }
        }
        assert_eq!(replay, "HELLO world!");
        // Same-place insertions: the later one goes after.
        docs.edit(&k, 4, op(0, 0, "1"), "a").unwrap();
        docs.edit(&k, 4, op(0, 0, "2"), "b").unwrap();
        assert_eq!(docs.text(&k).unwrap(), "12HELLO world!");
    }

    #[test]
    fn saves_wait_for_quiet_and_are_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let (mut docs, _) = recording();
        let k = key();
        docs.open(dir.path(), &k).unwrap();
        docs.edit(&k, 1, op(0, 0, "x"), "a").unwrap();
        let start = Instant::now();
        assert!(docs.save_due(dir.path(), start, false).is_empty());
        let later = start + SAVE_AFTER + Duration::from_millis(10);
        assert_eq!(docs.save_due(dir.path(), later, false), vec![k.clone()]);
        assert_eq!(
            std::fs::read_to_string(k.path(dir.path()).unwrap()).unwrap(),
            "x"
        );
        // Closing a clean document unloads it.
        docs.close(&k);
        assert!(!docs.is_open(&k));
    }

    #[test]
    fn an_outside_change_reloads_a_clean_note_and_is_kept_beside_a_dirty_one() {
        let dir = tempfile::tempdir().unwrap();
        let (mut docs, events) = recording();
        let k = key();
        let path = k.path(dir.path()).unwrap();
        projectfiles::write_atomic(&path, b"mine").unwrap();
        docs.open(dir.path(), &k).unwrap();

        // Clean: reloads.
        std::thread::sleep(Duration::from_millis(20));
        projectfiles::write_atomic(&path, b"theirs, longer").unwrap();
        docs.check_disk(dir.path(), "20261113-101500");
        assert_eq!(docs.text(&k).unwrap(), "theirs, longer");
        assert!(matches!(
            events.lock().unwrap().last(),
            Some(DocEvent::Reset { text, .. }) if text == "theirs, longer"
        ));

        // Dirty: the views' text wins, the outside one is kept.
        let rev = 2;
        docs.edit(&k, rev, op(0, 6, "MINE"), "a").unwrap();
        projectfiles::write_atomic(&path, b"outside again").unwrap();
        docs.check_disk(dir.path(), "20261113-101600");
        assert_eq!(docs.text(&k).unwrap(), "MINE, longer");
        let kept = projectfiles::file_path(
            dir.path(),
            &k.extension_id,
            &k.project_id,
            "notepad.external-20261113-101600.md",
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(kept).unwrap(), "outside again");
        assert!(matches!(
            events.lock().unwrap().last(),
            Some(DocEvent::Notice { .. })
        ));
        docs.save_due(dir.path(), Instant::now(), true);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "MINE, longer");
        // Its own save is not mistaken for an outside change.
        docs.check_disk(dir.path(), "20261113-101700");
        assert_eq!(docs.text(&k).unwrap(), "MINE, longer");
    }

    #[test]
    fn a_deleted_projects_open_note_is_saved_then_closed() {
        let dir = tempfile::tempdir().unwrap();
        let (mut docs, events) = recording();
        let k = key();
        docs.open(dir.path(), &k).unwrap();
        docs.edit(&k, 1, op(0, 0, "unsaved"), "a").unwrap();
        docs.close_where(dir.path(), |key| key.project_id == "p1", "deleted");
        assert!(!docs.is_open(&k));
        assert_eq!(
            std::fs::read_to_string(k.path(dir.path()).unwrap()).unwrap(),
            "unsaved"
        );
        assert!(matches!(
            events.lock().unwrap().last(),
            Some(DocEvent::Closed { reason, .. }) if reason == "deleted"
        ));
        // Edits to it now are refused, not resurrected.
        assert!(docs.edit(&k, 2, op(0, 0, "late"), "a").is_err());
    }

    #[test]
    fn a_very_long_note_edits_quickly() {
        let dir = tempfile::tempdir().unwrap();
        let (mut docs, _) = recording();
        let k = key();
        docs.open(dir.path(), &k).unwrap();
        let line = "Some text with $x^2$ and @attias2022 in it, and more words.\n";
        let big: String = line.repeat(1_048_576 / line.len() + 1);
        docs.edit(&k, 1, op(0, 0, &big), "a").unwrap();
        let started = Instant::now();
        let mut rev = 2;
        for i in 0..200 {
            let at = big.len() / 2 + i;
            rev = docs.edit(&k, rev, op(at, at, "x"), "a").unwrap().0;
        }
        // A debug build; a release build is many times faster.
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(docs.text(&k).unwrap().len(), big.len() + 200);
    }

    #[test]
    fn external_names_keep_the_extension() {
        assert_eq!(
            external_name("notepad.md", "20261113-101500"),
            "notepad.external-20261113-101500.md"
        );
        assert_eq!(
            external_name("a/b", "20261113-101500"),
            "a/b.external-20261113-101500"
        );
    }
}
