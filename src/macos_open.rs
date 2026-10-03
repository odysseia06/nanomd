//! macOS hands a file opened from Finder, or with `open`, to the app as an
//! "open documents" Apple Event rather than on the command line, and winit's
//! app delegate ignores that event (#23). This handles it instead.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{ClassType, DeclaredClass, declare_class, msg_send, msg_send_id, mutability, sel};
use objc2_foundation::{
    MainThreadMarker, NSAppleEventDescriptor, NSAppleEventManager, NSNotification,
    NSNotificationCenter, NSString,
};

/// A four-character Apple Event code, such as `aevt`.
const fn code(c: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*c)
}

/// Files that arrived before the window existed; `None` once
/// [`launch_file`] has taken them.
static QUEUED: Mutex<Option<Vec<PathBuf>>> = Mutex::new(Some(Vec::new()));

declare_class!(
    struct Opener;

    unsafe impl ClassType for Opener {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "NanomdOpener";
    }

    impl DeclaredClass for Opener {}

    unsafe impl Opener {
        // AppKit installs its own handler, which asks the app delegate, just
        // before this notification; one installed earlier would be replaced.
        #[method(willFinishLaunching:)]
        fn will_finish_launching(&self, _: &NSNotification) {
            unsafe {
                let _: () = msg_send![
                    &NSAppleEventManager::sharedAppleEventManager(),
                    setEventHandler: self,
                    andSelector: sel!(openDocuments:reply:),
                    forEventClass: code(b"aevt"),
                    andEventID: code(b"odoc")
                ];
            }
        }

        #[method(openDocuments:reply:)]
        fn open_documents(&self, event: &NSAppleEventDescriptor, _: &NSAppleEventDescriptor) {
            let paths = files(event);
            match QUEUED.lock().unwrap().as_mut() {
                Some(queued) => queued.extend(paths),
                None => paths.into_iter().for_each(new_window),
            }
        }
    }
);

/// The files in an "open documents" event: its direct object is a list of
/// file URLs.
fn files(event: &NSAppleEventDescriptor) -> Vec<PathBuf> {
    let list: Option<Retained<NSAppleEventDescriptor>> =
        unsafe { msg_send_id![event, paramDescriptorForKeyword: code(b"----")] };
    let Some(list) = list else {
        return Vec::new();
    };
    (1..=unsafe { list.numberOfItems() })
        .filter_map(|i| unsafe { list.descriptorAtIndex(i)?.fileURLValue()?.path() })
        .map(|p| PathBuf::from(p.to_string()))
        .collect()
}

/// Starts listening for files opened from Finder. Call before the event loop
/// starts.
pub fn listen() {
    let mtm = MainThreadMarker::new().expect("main() runs on the main thread");
    let this = mtm.alloc::<Opener>().set_ivars(());
    let opener: Retained<Opener> = unsafe { msg_send_id![super(this), init] };
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &opener,
            sel!(willFinishLaunching:),
            Some(&NSString::from_str(
                "NSApplicationWillFinishLaunchingNotification",
            )),
            None,
        );
    }
    // Neither the notification center nor the event manager keeps it alive.
    std::mem::forget(opener);
}

/// The file macOS launched the app to open, if any; it has arrived by the
/// time eframe creates the app. Any other file, then or later, opens in a
/// new window, as a double-click does on Windows and Linux.
pub fn launch_file() -> Option<PathBuf> {
    let mut files = QUEUED
        .lock()
        .unwrap()
        .take()
        .unwrap_or_default()
        .into_iter();
    let first = files.next();
    files.for_each(new_window);
    first
}

fn new_window(path: PathBuf) {
    let spawned = std::env::current_exe().and_then(|exe| Command::new(exe).arg(&path).spawn());
    if let Err(e) = spawned {
        eprintln!("nanomd: could not open {}: {e}", path.display());
    }
}
