//! The two places macOS talks to the app outside its window: files handed
//! over by Finder, and the menu bar.
//!
//! # Files from Finder
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
//!
//! # Menu bar
//!
//! Mac apps keep their commands in the menu bar at the top of the screen.
//! Neither egui nor eframe has an API for it, but winit already builds the
//! application menu (About, Hide, Quit with Cmd+Q) when the app launches;
//! [`install_menus`] adds File and Window menus beside it. Window's items go to
//! AppKit's own window actions; File's are queued the same way Finder's files
//! are, for [`take_menu_commands`] to hand to the app on its next frame.
//!
//! A Cmd shortcut bound to a menu item never reaches egui: AppKit offers each
//! key-down to the menu bar first, and winit's view does not claim it ahead of
//! the menu. So the menu is the one place those shortcuts act on macOS.

use std::path::PathBuf;
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSString,
};

use crate::app::AppCommand;

static OPENED_FILES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static MENU_COMMANDS: Mutex<Vec<AppCommand>> = Mutex::new(Vec::new());

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

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ViewarrMenuHandler"]
    struct MenuHandler;

    impl MenuHandler {
        #[unsafe(method(openFile:))]
        fn open_file(&self, _sender: Option<&AnyObject>) {
            queue_menu_command(AppCommand::Open);
        }

        #[unsafe(method(openFileInTab:))]
        fn open_file_in_tab(&self, _sender: Option<&AnyObject>) {
            queue_menu_command(AppCommand::OpenInTab);
        }

        #[unsafe(method(closeTabOrWindow:))]
        fn close_tab_or_window(&self, _sender: Option<&AnyObject>) {
            queue_menu_command(AppCommand::Close);
        }
    }
);

fn queue_menu_command(command: AppCommand) {
    MENU_COMMANDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(command);
}

/// Add File and Window menus to the menu bar. Call once, on the main thread,
/// after winit has built the application menu -- which it has by the time
/// eframe creates the app.
pub fn install_menus() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let menu_bar = app.mainMenu().unwrap_or_else(|| {
        let bar = NSMenu::new(mtm);
        app.setMainMenu(Some(&bar));
        bar
    });

    let this = MenuHandler::alloc(mtm).set_ivars(());
    let handler: Retained<MenuHandler> = unsafe { msg_send![super(this), init] };
    let handler_object: &AnyObject = &handler;

    let file = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("File"));
    file.addItem(&menu_item(mtm, "Open…", sel!(openFile:), "o", Some(handler_object)));
    file.addItem(&menu_item(mtm, "Open in Tab…", sel!(openFileInTab:), "t", Some(handler_object)));
    file.addItem(&NSMenuItem::separatorItem(mtm));
    file.addItem(&menu_item(mtm, "Close", sel!(closeTabOrWindow:), "w", Some(handler_object)));

    // No target: these go up the responder chain to the key window (or, for
    // Bring All to Front, the application), which is what makes them act on
    // whichever window is in front, header windows included.
    let window = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("Window"));
    window.addItem(&menu_item(mtm, "Minimize", sel!(performMiniaturize:), "m", None));
    window.addItem(&menu_item(mtm, "Zoom", sel!(performZoom:), "", None));
    window.addItem(&NSMenuItem::separatorItem(mtm));
    window.addItem(&menu_item(mtm, "Bring All to Front", sel!(arrangeInFront:), "", None));

    // File goes right after the application menu, Window at the end -- the
    // order every Mac app uses.
    let index = menu_bar.numberOfItems().min(1);
    menu_bar.insertItem_atIndex(&submenu_item(mtm, &file), index);
    menu_bar.addItem(&submenu_item(mtm, &window));
    // Registering the Window menu is what makes AppKit list the open windows
    // at the bottom of it.
    app.setWindowsMenu(Some(&window));

    // A menu item does not keep its target alive, and this one is needed for
    // the life of the process.
    std::mem::forget(handler);
}

/// Every File-menu command chosen since the last call, in order.
pub fn take_menu_commands() -> Vec<AppCommand> {
    std::mem::take(
        &mut *MENU_COMMANDS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// A menu item with a Cmd+`key` equivalent (none if `key` is empty), sending
/// `action` to `target`, or up the responder chain when that is `None`.
fn menu_item(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    key: &str,
    target: Option<&AnyObject>,
) -> Retained<NSMenuItem> {
    // SAFETY: every action passed in names a method that takes the sender,
    // either on MenuHandler above or on AppKit's own responders.
    unsafe {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key),
        );
        item.setTarget(target);
        item
    }
}

/// A menu-bar entry that opens `menu`.
fn submenu_item(mtm: MainThreadMarker, menu: &NSMenu) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm);
    item.setSubmenu(Some(menu));
    item
}
