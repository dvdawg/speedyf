//! macOS quitting from outside the app - the Dock's Quit, logout, restart,
//! shut down, a quit Apple event - by way of the same unsaved-changes flow as
//! Cmd+Q.
//!
//! tao's application delegate does not implement
//! `applicationShouldTerminate:`, so AppKit takes the default answer and the
//! process ends at once, edits and all. This adds the method to that
//! delegate. It answers "later", asks every window to close exactly as the
//! Quit menu item does, and replies once the windows have answered: yes when
//! the last one has gone (`RunEvent::ExitRequested`), no when one is
//! cancelled, which also makes macOS call off a logout or restart.
//!
//! The reply goes through the main dispatch queue. It must not be sent from
//! inside a tao event callback: terminating runs `applicationWillTerminate:`,
//! which re-enters tao's event handler and would deadlock on its lock.
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{class, msg_send, sel};
use std::sync::OnceLock;

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// `NSApplicationTerminateReply`.
const TERMINATE_NOW: usize = 1;
const TERMINATE_LATER: usize = 2;

type Handler = extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize;

/// Adds `applicationShouldTerminate:` to the application delegate. Main
/// thread, once the event loop exists (from `setup`).
pub fn install(app: &tauri::AppHandle) {
    if APP.set(app.clone()).is_err() {
        return;
    }
    // SAFETY: called on the main thread; the delegate is tao's live
    // NSApplication delegate, and the type encoding matches the handler:
    // NSUInteger return, self, _cmd, one object argument.
    unsafe {
        let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut AnyObject = msg_send![ns_app, delegate];
        let Some(delegate) = delegate.as_ref() else {
            log::warn!("no application delegate; a system quit skips the unsaved check");
            return;
        };
        let cls: *const AnyClass = delegate.class();
        let imp = std::mem::transmute::<Handler, Imp>(should_terminate);
        let added = objc2::ffi::class_addMethod(
            cls.cast_mut(),
            sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        );
        if !added.as_bool() {
            log::warn!("application delegate already answers applicationShouldTerminate:");
        }
    }
}

extern "C-unwind" fn should_terminate(_: &AnyObject, _: Sel, _: *mut AnyObject) -> usize {
    match APP.get() {
        Some(app) if crate::windows::system_quit(app) => TERMINATE_LATER,
        _ => TERMINATE_NOW,
    }
}

/// Answers the pending terminate request: `true` lets the app end (and a
/// logout go on), `false` calls it off.
pub fn reply(terminate: bool) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        // SAFETY: on the main thread, and AppKit is waiting for this reply.
        unsafe {
            let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![ns_app, replyToApplicationShouldTerminate: Bool::new(terminate)];
        }
    });
}
