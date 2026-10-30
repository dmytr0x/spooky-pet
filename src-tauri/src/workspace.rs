use std::ptr::NonNull;
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{NSNotification, NSObjectProtocol};
use tauri::{AppHandle, Manager};

use crate::AppState;

#[allow(dead_code)]
struct ObserverGuard(Retained<ProtocolObject<dyn NSObjectProtocol>>);
unsafe impl Send for ObserverGuard {}
unsafe impl Sync for ObserverGuard {}

static OBSERVER: OnceLock<ObserverGuard> = OnceLock::new();

pub fn start(app: &AppHandle) {
    store_frontmost(app);
    observe(app.clone());
}

pub fn frontmost() -> Option<(String, String)> {
    let workspace = NSWorkspace::sharedWorkspace();
    let app = workspace.frontmostApplication()?;
    identity(&app)
}

fn observe(app: AppHandle) {
    let workspace = NSWorkspace::sharedWorkspace();
    let center = workspace.notificationCenter();
    let handle = app.clone();
    let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        if store_frontmost(&handle) {
            crate::on_frontmost_changed(&handle);
        }
    });
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            None,
            &block,
        )
    };
    let _ = OBSERVER.set(ObserverGuard(observer));
}

fn store_frontmost(app: &AppHandle) -> bool {
    let Some((name, bundle_id)) = frontmost() else {
        return false;
    };
    if bundle_id == app.config().identifier {
        return false;
    }
    eprintln!("frontmost: {name} / {bundle_id}");
    let state_lock = app.state::<std::sync::Mutex<AppState>>();
    let Ok(mut state) = state_lock.lock() else {
        return false;
    };
    if state.frontmost_name == name && state.frontmost_bundle_id == bundle_id {
        return false;
    }
    state.frontmost_name = name;
    state.frontmost_bundle_id = bundle_id;
    true
}

fn identity(app: &NSRunningApplication) -> Option<(String, String)> {
    let name = app.localizedName().map(|s| s.to_string())?;
    let bundle_id = app.bundleIdentifier().map(|s| s.to_string())?;
    Some((name, bundle_id))
}
