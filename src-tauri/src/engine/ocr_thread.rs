//! The OCR thread: text recognition off the PDFium worker.
//!
//! Vision's first call of a session loads its model and can take tens of
//! seconds; a page after that ~0.4 s. On the worker thread either one held
//! every render behind it. Now the worker only renders the page (PDFium
//! must stay on the worker) and hands the PNG here, through a bounded
//! channel, then goes on with its queue. The result is written to the
//! document's OCR cache at once - so a reading finished just before the app
//! quits is not lost - and handed back through `on_done`, which the engine
//! turns into an idle `Work::OcrDone` that resumes the library job waiting
//! for it (`worker::do_ocr_done`).
//!
//! The thread is spawned on the first request, so a build or a session that
//! never OCRs never starts it, and it ends when the service is dropped,
//! after reading what is already queued.

use super::ocr::OcrLine;
use crate::library::ocrtext::OcrCache;
use std::path::PathBuf;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::Instant;

/// Requests queued at most. The worker keeps one in flight per document and
/// only the scan and the text pass ask, so this is never reached in
/// practice; a full channel is waited out like a busy document.
const CAPACITY: usize = 4;

/// One rendered page for the OCR thread to read.
pub struct OcrRequest {
    pub path: PathBuf,
    /// The content fingerprint the cache is keyed by.
    pub fingerprint: Option<String>,
    pub app_data_dir: PathBuf,
    pub page: u32,
    pub png: Vec<u8>,
}

/// A page read, or not: `lines` is `None` when Vision failed on it, which
/// is cached too, so it is not retried.
#[derive(Debug, Clone)]
pub struct OcrDone {
    pub path: PathBuf,
    pub page: u32,
    pub lines: Option<Vec<OcrLine>>,
}

pub type Recognizer = Arc<dyn Fn(&[u8]) -> Option<Vec<OcrLine>> + Send + Sync>;
pub type OnDone = Arc<dyn Fn(OcrDone) + Send + Sync>;

/// Why a request was not taken.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The channel is full: wait for a reading to come back, then ask again.
    Busy,
    /// No OCR thread (it could not be spawned): read nothing.
    Unavailable,
}

pub struct OcrService {
    sender: Option<SyncSender<OcrRequest>>,
    recognize: Recognizer,
    on_done: OnDone,
    spawn_failed: bool,
}

impl OcrService {
    pub fn new(recognize: Recognizer, on_done: OnDone) -> Self {
        OcrService {
            sender: None,
            recognize,
            on_done,
            spawn_failed: false,
        }
    }

    /// Queue a page for reading. Never blocks.
    pub fn submit(&mut self, request: OcrRequest) -> Result<(), Refused> {
        let Some(sender) = self.sender() else {
            return Err(Refused::Unavailable);
        };
        match sender.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(Refused::Busy),
            Err(TrySendError::Disconnected(_)) => {
                // The thread is gone; a later request may start another.
                self.sender = None;
                Err(Refused::Unavailable)
            }
        }
    }

    fn sender(&mut self) -> Option<&SyncSender<OcrRequest>> {
        if self.sender.is_none() && !self.spawn_failed {
            let (sender, receiver) = mpsc::sync_channel::<OcrRequest>(CAPACITY);
            let recognize = Arc::clone(&self.recognize);
            let on_done = Arc::clone(&self.on_done);
            let spawned = std::thread::Builder::new()
                .name("speedyf-ocr".into())
                .spawn(move || {
                    lower_priority();
                    while let Ok(request) = receiver.recv() {
                        on_done(read(&recognize, request));
                    }
                });
            match spawned {
                Ok(_) => self.sender = Some(sender),
                Err(error) => {
                    log::warn!("cannot start the OCR thread: {error}");
                    self.spawn_failed = true;
                }
            }
        }
        self.sender.as_ref()
    }
}

/// Read one page and cache the reading. A panic in the recognizer is a
/// failed reading, never a lost one: a job waits for every answer.
fn read(recognize: &Recognizer, request: OcrRequest) -> OcrDone {
    let started = Instant::now();
    let lines = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| recognize(&request.png)))
        .unwrap_or(None);
    log::info!(
        "ocr: {} page {} read {} lines in {:?}",
        request.path.display(),
        request.page + 1,
        lines.as_ref().map_or(0, Vec::len),
        started.elapsed()
    );
    let mut cache = OcrCache::load(
        &request.app_data_dir,
        &request.path,
        request.fingerprint.as_deref(),
    );
    cache.insert(request.page, lines.clone());
    cache.save(&request.app_data_dir, &request.path);
    OcrDone {
        path: request.path,
        page: request.page,
        lines,
    }
}

/// Run below the worker: OCR is background work and should never take a
/// core from a render. macOS schedules by quality of service.
fn lower_priority() {
    #[cfg(target_os = "macos")]
    // SAFETY: sets the calling thread's own QoS class; no pointers involved.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn request(dir: &std::path::Path, page: u32) -> OcrRequest {
        OcrRequest {
            path: dir.join("scan.pdf"),
            fingerprint: Some("fp".into()),
            app_data_dir: dir.to_path_buf(),
            page,
            png: vec![1, 2, 3],
        }
    }

    #[test]
    fn readings_are_cached_and_handed_back_and_a_panic_is_a_failed_reading() {
        let dir = tempfile::tempdir().unwrap();
        let (done_tx, done_rx) = channel();
        let done_tx = parking_lot::Mutex::new(done_tx);
        let mut service = OcrService::new(
            Arc::new(|png: &[u8]| {
                if png.is_empty() {
                    panic!("bad image");
                }
                Some(vec![OcrLine {
                    text: "Arthur-Merlin Games".into(),
                    confidence: 1.0,
                    bbox: [0.1, 0.1, 0.5, 0.03],
                }])
            }),
            Arc::new(move |done| done_tx.lock().send(done).unwrap()),
        );
        service.submit(request(dir.path(), 0)).unwrap();
        let mut broken = request(dir.path(), 1);
        broken.png.clear();
        service.submit(broken).unwrap();

        let first = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(first.page, 0);
        assert_eq!(first.lines.as_ref().unwrap()[0].text, "Arthur-Merlin Games");
        let second = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!((second.page, second.lines), (1, None));

        // Written through before the answer went back.
        let cache = OcrCache::load(dir.path(), &dir.path().join("scan.pdf"), Some("fp"));
        assert!(cache.get(0).unwrap().is_some());
        assert!(cache.get(1).unwrap().is_none());
    }

    #[test]
    fn a_full_channel_says_busy_rather_than_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = channel::<()>();
        let gate_rx = parking_lot::Mutex::new(gate_rx);
        let mut service = OcrService::new(
            Arc::new(move |_: &[u8]| {
                let _ = gate_rx.lock().recv();
                Some(Vec::new())
            }),
            Arc::new(|_| {}),
        );
        let mut refused = None;
        for page in 0..(CAPACITY as u32 + 3) {
            if let Err(why) = service.submit(request(dir.path(), page)) {
                refused = Some(why);
                break;
            }
        }
        assert_eq!(refused, Some(Refused::Busy));
        drop(gate_tx);
    }
}
