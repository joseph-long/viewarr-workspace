//! Files handed to the app by Finder rather than by the command line.
//!
//! Double-clicking a document, dropping one on the Dock icon, or `open -a`
//! never puts the path in argv: LaunchServices starts the app (or reuses the
//! running one) and sends it an "open documents" Apple Event. winit's
//! application delegate does not answer that event, so without this AppKit
//! falls back to its own document machinery, which knows nothing of FITS and
//! shows a "cannot open files" alert instead.
//!
//! The handler has to be registered after AppKit installs its default Apple
//! Event handlers, which `-[NSApplication finishLaunching]` does first thing,
//! but before that same method dispatches the event that launched the app.
//! The will-finish-launching notification is posted in exactly that gap, and
//! observing it needs nothing from eframe or winit.
//!
//! Paths are queued rather than opened here, since the launch event arrives
//! before the app (and its tabs) exist; [`take_opened_files`] hands them over
//! once it does. Nothing here knows about formats — whatever the bundle's
//! Info.plist lets Finder send is passed through, and the app decides.

use std::path::PathBuf;
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSString,
};

static OPENED_FILES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Four-character codes from the Apple Event headers, spelled out rather than
/// linked: `kCoreEventClass`, `kAEOpenDocuments` and `keyDirectObject`.
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ViewarrOpenDocumentsHandler"]
    struct OpenDocumentsHandler;

    impl OpenDocumentsHandler {
        #[unsafe(method(applicationWillFinishLaunching:))]
        fn will_finish_launching(&self, _notification: &NSNotification) {
            // SAFETY: the selector names the method below, whose signature is
            // the one NSAppleEventManager calls handlers with.
            unsafe {
                NSAppleEventManager::sharedAppleEventManager()
                    .setEventHandler_andSelector_forEventClass_andEventID(
                        self,
                        sel!(handleOpenDocuments:withReplyEvent:),
                        CORE_EVENT_CLASS,
                        OPEN_DOCUMENTS,
                    );
            }
        }

        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn handle_open_documents(
            &self,
            event: &NSAppleEventDescriptor,
            _reply: &NSAppleEventDescriptor,
        ) {
            let Some(direct) = event.paramDescriptorForKeyword(DIRECT_OBJECT) else {
                return;
            };
            // Finder always sends a list, but a lone file is a valid event too;
            // a non-list descriptor reports no items.
            let items: Vec<_> = match direct.numberOfItems() {
                0 => vec![direct],
                count => (1..=count)
                    .filter_map(|index| direct.descriptorAtIndex(index))
                    .collect(),
            };
            let paths = items
                .iter()
                .filter_map(|item| item.fileURLValue())
                .filter_map(|url| url.to_file_path());
            OPENED_FILES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(paths);
        }
    }
);

/// Start listening for files Finder asks the app to open. Call once, on the
/// main thread, before the event loop runs.
pub fn install_open_handler() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let this = OpenDocumentsHandler::alloc(mtm).set_ivars(());
    let handler: Retained<OpenDocumentsHandler> = unsafe { msg_send![super(this), init] };
    // SAFETY: the selector is implemented above and takes the notification.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &handler,
            sel!(applicationWillFinishLaunching:),
            Some(&NSString::from_str(
                "NSApplicationWillFinishLaunchingNotification",
            )),
            None,
        );
    }
    // Neither the notification center nor the Apple Event manager keeps its
    // handler alive, and this one is needed for the life of the process.
    std::mem::forget(handler);
}

/// Every file Finder has asked to open since the last call, in arrival order.
pub fn take_opened_files() -> Vec<PathBuf> {
    std::mem::take(
        &mut *OPENED_FILES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}
