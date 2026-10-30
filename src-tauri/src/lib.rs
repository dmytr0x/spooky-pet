mod llm;
mod settings;
mod workspace;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow,
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_nspanel::{
    CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt, tauri_panel,
};

use settings::Settings;

tauri_panel! {
    panel!(OverlayPanel {
        config: {
            can_become_key_window: false,
            is_floating_panel: true
        }
    })
}

const OVERLAY_LABEL: &str = "overlay";
const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");
const SPAWN_MARGIN_PT: f64 = 24.0;
const EVENT_STATE: &str = "pet://state";
const EVENT_JOKE: &str = "pet://joke";
const EVENT_CLEAR: &str = "pet://clear";
const EVENT_SOUND: &str = "pet://sound";
const WANDER_TICK: Duration = Duration::from_millis(50);
const WANDER_SPEED_MIN: f64 = 20.0;
const WANDER_SPEED_MAX: f64 = 35.0;
const HEADING_MIN_SECS: f64 = 2.0;
const HEADING_MAX_SECS: f64 = 5.0;
const DRAG_RESUME: Duration = Duration::from_secs(3);
const CLICK_COOLDOWN: Duration = Duration::from_secs(10);
const LURK: Duration = Duration::from_millis(700);
const BUBBLE: Duration = Duration::from_secs(8);
const CHUNK_LIMIT: u32 = 64;
const APP_SWITCH_DEBOUNCE: Duration = Duration::from_secs(3);
const IDLE_WIDTH: f64 = 160.0;
const IDLE_HEIGHT: f64 = 160.0;
const JOKE_WIDTH: f64 = 320.0;
const JOKE_HEIGHT: f64 = 320.0;

pub(crate) struct AppState {
    hidden: bool,
    muted: bool,
    joke_showing: bool,
    in_flight: bool,
    last_joke_at: Option<Instant>,
    last_click_at: Option<Instant>,
    joke_gen: u64,
    app_switch_gen: u64,
    phase: HecklePhase,
    wander_vx: f64,
    wander_vy: f64,
    wander_paused: bool,
    resume_at: Option<Instant>,
    last_repick_at: Instant,
    next_repick_in: Duration,
    dragging: bool,
    drag_press_seen: bool,
    wander_rng: Rng,
    frontmost_name: String,
    frontmost_bundle_id: String,
    settings: Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HecklePhase {
    Idle,
    Lurking,
    Talking,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Trigger {
    Click,
    Timer,
    AppSwitch,
}

impl Trigger {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "click" => Some(Self::Click),
            "timer" => Some(Self::Timer),
            "app_switch" => Some(Self::AppSwitch),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Timer => "timer",
            Self::AppSwitch => "app_switch",
        }
    }
}

impl AppState {
    fn new(settings: Settings) -> Self {
        let mut wander_rng = Rng::from_entropy();
        let (wander_vx, wander_vy, next_repick_in) = pick_heading(&mut wander_rng);
        Self {
            hidden: false,
            muted: false,
            joke_showing: false,
            in_flight: false,
            last_joke_at: None,
            last_click_at: None,
            joke_gen: 0,
            app_switch_gen: 0,
            phase: HecklePhase::Idle,
            wander_vx,
            wander_vy,
            wander_paused: false,
            resume_at: None,
            last_repick_at: Instant::now(),
            next_repick_in,
            dragging: false,
            drag_press_seen: false,
            wander_rng,
            frontmost_name: String::new(),
            frontmost_bundle_id: String::new(),
            settings,
        }
    }

    fn skip_work(&self) -> bool {
        self.hidden
    }

    fn pet_state(&self) -> &'static str {
        match self.phase {
            HecklePhase::Lurking => "lurking",
            HecklePhase::Talking => "talking",
            HecklePhase::Idle if self.muted => "muted",
            HecklePhase::Idle => "idle",
        }
    }

    fn sync_wander_paused(&mut self) {
        self.wander_paused = self.hidden || self.muted || self.joke_showing;
    }

    fn wander_should_move(&self, now: Instant) -> bool {
        if self.wander_paused || self.dragging {
            return false;
        }
        match self.resume_at {
            Some(at) if now < at => false,
            _ => true,
        }
    }

    fn set_dragging_at(&mut self, dragging: bool, now: Instant) {
        if dragging {
            self.dragging = true;
            self.drag_press_seen = false;
            self.resume_at = None;
            return;
        }
        if self.dragging {
            self.dragging = false;
            self.drag_press_seen = false;
            self.resume_at = Some(now + DRAG_RESUME);
        }
    }

    fn maybe_repick(&mut self, now: Instant) {
        let Some(elapsed) = now.checked_duration_since(self.last_repick_at) else {
            return;
        };
        if elapsed < self.next_repick_in {
            return;
        }
        let (vx, vy, interval) = pick_heading(&mut self.wander_rng);
        self.wander_vx = vx;
        self.wander_vy = vy;
        self.last_repick_at = now;
        self.next_repick_in = interval;
    }
}

#[derive(Serialize)]
struct FrontmostApp {
    name: String,
    bundle_id: String,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_nspanel::init())
        .invoke_handler(tauri::generate_handler![
            set_hidden,
            set_muted,
            set_dragging,
            heckle,
            dismiss_joke,
            frontmost_app
        ])
        .setup(|app| {
            let settings = settings::load_or_init(app)?;
            app.manage(Mutex::new(AppState::new(settings)));
            setup_overlay(app)?;
            setup_tray(app)?;
            workspace::start(app.handle());
            start_wander(app.handle().clone());
            start_heckle_timer(app.handle().clone());
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_overlay(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let window = app
        .get_webview_window(OVERLAY_LABEL)
        .ok_or("overlay window missing")?;

    place_lower_right(&window)?;

    let panel = window
        .to_panel::<OverlayPanel>()
        .map_err(|err| format!("convert overlay to NSPanel: {err:?}"))?;

    panel.set_level(PanelLevel::Floating.value());
    panel.add_style_mask(StyleMask::empty().nonactivating_panel().into())?;
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .can_join_all_spaces()
            .full_screen_auxiliary()
            .ignores_cycle()
            .into(),
    );
    panel.set_hides_on_deactivate(false);
    panel.set_has_shadow(false);
    panel.set_opaque(false);
    panel.set_transparent(true);
    Ok(())
}

fn place_lower_right(window: &WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = window.primary_monitor()? else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    let work = monitor.work_area();
    let win = window.outer_size()?;
    let margin = (SPAWN_MARGIN_PT * scale).round() as i32;
    let x = work.position.x + work.size.width as i32 - win.width as i32 - margin;
    let y = work.position.y + work.size.height as i32 - win.height as i32 - margin;
    window.set_position(PhysicalPosition::new(x, y))?;
    Ok(())
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_hide = MenuItem::with_id(app, "show_hide", "Hide", true, None::<&str>)?;
    let mute = CheckMenuItem::with_id(app, "mute", "Mute", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_hide, &mute, &quit])?;

    let show_hide_item = show_hide.clone();
    let mute_item = mute.clone();
    let icon = Image::from_bytes(TRAY_ICON)?;

    TrayIconBuilder::with_id("tray")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Spooky Pet")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show_hide" => {
                let next = match with_state(app, |state| !state.hidden) {
                    Ok(hidden) => hidden,
                    Err(_) => return,
                };
                if apply_hidden(app, next).is_ok() {
                    let label = if next { "Show" } else { "Hide" };
                    let _ = show_hide_item.set_text(label);
                }
            }
            "mute" => {
                let next = match with_state(app, |state| !state.muted) {
                    Ok(muted) => muted,
                    Err(_) => return,
                };
                if apply_muted(app, next).is_ok() {
                    let _ = mute_item.set_checked(next);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

fn with_state<T>(app: &AppHandle, f: impl FnOnce(&mut AppState) -> T) -> Result<T, String> {
    let state = app.state::<Mutex<AppState>>();
    let mut guard = state.lock().map_err(|err| err.to_string())?;
    Ok(f(&mut guard))
}

fn reload_settings(app: &AppHandle) -> Result<Settings, String> {
    let settings = settings::load_or_init(app)?;
    with_state(app, |state| {
        state.settings = settings.clone();
    })?;
    Ok(settings)
}

/// Re-read settings, then decide whether SFX may play. Heckle/sound call this path.
fn sound_allowed(app: &AppHandle) -> Result<bool, String> {
    let settings = reload_settings(app)?;
    with_state(app, |state| {
        sfx_allowed(settings.sound, state.muted, state.hidden)
    })
}

fn sfx_allowed(sound: bool, muted: bool, hidden: bool) -> bool {
    sound && !muted && !hidden
}

fn maybe_play_sound(app: &AppHandle, kind: &'static str) {
    if !sound_allowed(app).unwrap_or(false) {
        return;
    }
    if let Err(err) = app.emit(EVENT_SOUND, SoundPayload { kind }) {
        eprintln!("sound: {err}");
    }
}

fn apply_hidden(app: &AppHandle, hidden: bool) -> Result<(), String> {
    let (phase, token) = with_state(app, |state| {
        state.hidden = hidden;
        state.sync_wander_paused();
        (state.phase, state.joke_gen)
    })?;
    let panel = app
        .get_webview_panel(OVERLAY_LABEL)
        .map_err(|err| format!("{err:?}"))?;
    if hidden {
        match phase {
            HecklePhase::Talking => finish_joke(app, token)?,
            HecklePhase::Lurking => abort_heckle(app, token)?,
            HecklePhase::Idle => {}
        }
        panel.hide();
    } else {
        panel.show();
        emit_pet_state(app)?;
    }
    Ok(())
}

fn apply_muted(app: &AppHandle, muted: bool) -> Result<(), String> {
    let hidden = with_state(app, |state| {
        state.muted = muted;
        state.sync_wander_paused();
        state.hidden
    })?;
    if !hidden {
        emit_pet_state(app)?;
    }
    Ok(())
}

fn emit_pet_state(app: &AppHandle) -> Result<(), String> {
    let payload = with_state(app, |state| state.pet_state())?;
    app.emit(EVENT_STATE, payload)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn set_hidden(app: AppHandle, hidden: bool) -> Result<(), String> {
    apply_hidden(&app, hidden)
}

#[tauri::command]
fn set_muted(app: AppHandle, muted: bool) -> Result<(), String> {
    apply_muted(&app, muted)
}

#[tauri::command]
fn set_dragging(app: AppHandle, dragging: bool) -> Result<(), String> {
    with_state(&app, |state| {
        state.set_dragging_at(dragging, Instant::now());
    })
}

#[tauri::command]
fn heckle(app: AppHandle, trigger: String) -> Result<(), String> {
    begin_heckle(app, trigger)
}

#[tauri::command]
fn dismiss_joke(app: AppHandle) -> Result<(), String> {
    let token = with_state(&app, |state| state.joke_gen)?;
    finish_joke(&app, token)
}

fn begin_heckle(app: AppHandle, trigger: String) -> Result<(), String> {
    begin_heckle_with_joke(app, trigger, None)
}

fn begin_heckle_with_joke(
    app: AppHandle,
    trigger: String,
    joke: Option<String>,
) -> Result<(), String> {
    let Some(trigger) = Trigger::parse(&trigger) else {
        return Ok(());
    };
    let settings = reload_settings(&app)?;
    let cooldown = Duration::from_secs(settings.cooldown_seconds);
    let now = Instant::now();
    let started = with_state(&app, |state| {
        if heckle_blocked(state, trigger, now, cooldown) {
            return false;
        }
        state.in_flight = true;
        state.joke_showing = true;
        state.phase = HecklePhase::Lurking;
        state.joke_gen = state.joke_gen.wrapping_add(1);
        if trigger == Trigger::Click {
            state.last_click_at = Some(now);
        }
        state.sync_wander_paused();
        true
    })?;
    if !started {
        return Ok(());
    }
    emit_pet_state(&app)?;
    maybe_play_sound(&app, "lurk");
    let token = with_state(&app, |state| state.joke_gen)?;
    tauri::async_runtime::spawn(async move {
        if let Err(err) = run_heckle_pipeline(app, trigger, token, joke).await {
            eprintln!("heckle: {err}");
        }
    });
    Ok(())
}

fn heckle_blocked(state: &AppState, trigger: Trigger, now: Instant, cooldown: Duration) -> bool {
    if state.skip_work() || state.in_flight {
        return true;
    }
    match trigger {
        Trigger::Click => still_cooling(state.last_click_at, now, CLICK_COOLDOWN),
        Trigger::Timer | Trigger::AppSwitch => {
            state.muted || state.joke_showing || still_cooling(state.last_joke_at, now, cooldown)
        }
    }
}

fn still_cooling(at: Option<Instant>, now: Instant, for_: Duration) -> bool {
    at.and_then(|at| now.checked_duration_since(at))
        .is_some_and(|elapsed| elapsed < for_)
}

async fn request_capped_joke(
    settings: &Settings,
    trigger: Trigger,
    name: &str,
    bundle_id: &str,
) -> String {
    llm::hard_cap(&llm::complete(settings, trigger.as_str(), name, bundle_id).await)
}

fn timer_heckle_blocked(app: &AppHandle) -> Result<bool, String> {
    let settings = reload_settings(app)?;
    let cooldown = Duration::from_secs(settings.cooldown_seconds);
    let now = Instant::now();
    with_state(app, |state| {
        heckle_blocked(state, Trigger::Timer, now, cooldown)
    })
}

async fn prefetch_timer_joke(app: &AppHandle) -> Result<Option<String>, String> {
    if timer_heckle_blocked(app)? {
        return Ok(None);
    }
    let (settings, name, bundle_id) = with_state(app, |state| {
        (
            state.settings.clone(),
            state.frontmost_name.clone(),
            state.frontmost_bundle_id.clone(),
        )
    })?;
    Ok(Some(
        request_capped_joke(&settings, Trigger::Timer, &name, &bundle_id).await,
    ))
}

async fn run_heckle_pipeline(
    app: AppHandle,
    trigger: Trigger,
    token: u64,
    prefetched: Option<String>,
) -> Result<(), String> {
    sleep_for(LURK).await;

    if heckle_stale_or_hidden(&app, token)? {
        abort_heckle(&app, token)?;
        return Ok(());
    }

    let settings = with_state(&app, |state| state.settings.clone())?;
    let text = match prefetched {
        Some(text) => text,
        None => {
            let (name, bundle_id) = with_state(&app, |state| {
                (
                    state.frontmost_name.clone(),
                    state.frontmost_bundle_id.clone(),
                )
            })?;
            request_capped_joke(&settings, trigger, &name, &bundle_id).await
        }
    };

    if heckle_stale_or_hidden(&app, token)? {
        abort_heckle(&app, token)?;
        return Ok(());
    }

    let still = with_state(&app, |state| {
        if state.joke_gen != token {
            return false;
        }
        state.phase = HecklePhase::Talking;
        true
    })?;
    if !still {
        abort_heckle(&app, token)?;
        return Ok(());
    }
    if let Err(err) = grow_overlay(&app) {
        abort_heckle(&app, token)?;
        return Err(err);
    }
    let chunk_delay = Duration::from_secs(settings.joke_chunk_delay_seconds());
    if let Err(err) = app.emit(
        EVENT_JOKE,
        JokePayload {
            text,
            font_size: settings.joke_font_size(),
            popup_opacity: settings.joke_popup_opacity(),
            chunk_delay_ms: chunk_delay.as_millis() as u64,
        },
    ) {
        abort_heckle(&app, token)?;
        return Err(err.to_string());
    }
    emit_pet_state(&app)?;
    maybe_play_sound(&app, "speak");

    sleep_for(joke_display_limit(chunk_delay)).await;
    finish_joke(&app, token)
}

fn joke_display_limit(chunk_delay: Duration) -> Duration {
    let delay = if chunk_delay.is_zero() {
        BUBBLE
    } else {
        chunk_delay
    };
    delay.saturating_mul(CHUNK_LIMIT)
}

fn heckle_stale_or_hidden(app: &AppHandle, token: u64) -> Result<bool, String> {
    with_state(app, |state| state.hidden || state.joke_gen != token)
}

fn abort_heckle(app: &AppHandle, token: u64) -> Result<(), String> {
    let outcome = with_state(app, |state| {
        if state.joke_gen != token {
            return None;
        }
        state.joke_gen = state.joke_gen.wrapping_add(1);
        state.phase = HecklePhase::Idle;
        state.joke_showing = false;
        state.in_flight = false;
        state.sync_wander_paused();
        Some(state.hidden)
    })?;
    let Some(hidden) = outcome else {
        return Ok(());
    };
    let _ = app.emit(EVENT_CLEAR, ());
    if !hidden {
        emit_pet_state(app)?;
    }
    Ok(())
}

fn finish_joke(app: &AppHandle, token: u64) -> Result<(), String> {
    let outcome = with_state(app, |state| {
        if state.joke_gen != token {
            return None;
        }
        if state.phase == HecklePhase::Idle && !state.joke_showing && !state.in_flight {
            return None;
        }
        state.joke_gen = state.joke_gen.wrapping_add(1);
        state.phase = HecklePhase::Idle;
        state.joke_showing = false;
        state.in_flight = false;
        state.last_joke_at = Some(Instant::now());
        state.sync_wander_paused();
        Some(state.hidden)
    })?;
    let Some(hidden) = outcome else {
        return Ok(());
    };
    app.emit(EVENT_CLEAR, ()).map_err(|err| err.to_string())?;
    if !hidden {
        emit_pet_state(app)?;
    }
    shrink_overlay(app)
}

fn grow_overlay(app: &AppHandle) -> Result<(), String> {
    resize_overlay(app, JOKE_WIDTH, JOKE_HEIGHT)
}

fn shrink_overlay(app: &AppHandle) -> Result<(), String> {
    resize_overlay(app, IDLE_WIDTH, IDLE_HEIGHT)
}

fn resize_overlay(app: &AppHandle, logical_w: f64, logical_h: f64) -> Result<(), String> {
    let window = app
        .get_webview_window(OVERLAY_LABEL)
        .ok_or("overlay window missing")?;
    let scale = window.scale_factor().map_err(|err| err.to_string())?;
    if scale <= 0.0 {
        return Ok(());
    }
    let old_pos = window.outer_position().map_err(|err| err.to_string())?;
    let old_size = window.outer_size().map_err(|err| err.to_string())?;
    let new_w_px = logical_w * scale;
    let new_h_px = logical_h * scale;
    let (x, y) = resize_origin(
        f64::from(old_pos.x),
        f64::from(old_pos.y),
        f64::from(old_size.width),
        f64::from(old_size.height),
        new_w_px,
        new_h_px,
    );
    window
        .set_size(LogicalSize::new(logical_w, logical_h))
        .map_err(|err| err.to_string())?;

    let frame = match window.current_monitor().map_err(|err| err.to_string())? {
        Some(monitor) => {
            let work = monitor.work_area();
            Frame {
                x: f64::from(work.position.x),
                y: f64::from(work.position.y),
                w: f64::from(work.size.width),
                h: f64::from(work.size.height),
            }
        }
        None => {
            window
                .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
                .map_err(|err| err.to_string())?;
            return Ok(());
        }
    };
    let (x, y) = clamp_origin(x, y, new_w_px, new_h_px, frame);
    window
        .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn resize_origin(x: f64, y: f64, old_w: f64, old_h: f64, new_w: f64, new_h: f64) -> (f64, f64) {
    (x - (new_w - old_w) / 2.0, y - (new_h - old_h))
}

fn clamp_origin(x: f64, y: f64, overlay_w: f64, overlay_h: f64, work: Frame) -> (f64, f64) {
    let max_x = (work.x + work.w - overlay_w).max(work.x);
    let max_y = (work.y + work.h - overlay_h).max(work.y);
    (x.clamp(work.x, max_x), y.clamp(work.y, max_y))
}

fn start_heckle_timer(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut rng = Rng::from_entropy();
        loop {
            let delay = match reload_settings(&app) {
                Ok(settings) => pick_timer_delay(&settings, &mut rng),
                Err(_) => pick_timer_delay(&Settings::default(), &mut rng),
            };
            sleep_for(delay).await;
            let joke = match prefetch_timer_joke(&app).await {
                Ok(joke) => joke,
                Err(err) => {
                    eprintln!("timer joke: {err}");
                    continue;
                }
            };
            let Some(joke) = joke else {
                continue;
            };
            if let Err(err) =
                begin_heckle_with_joke(app.clone(), Trigger::Timer.as_str().to_string(), Some(joke))
            {
                eprintln!("timer heckle: {err}");
            }
        }
    });
}

pub(crate) fn on_frontmost_changed(app: &AppHandle) {
    let Some(snapshot) = workspace::frontmost() else {
        return;
    };
    if snapshot.1 == app.config().identifier {
        return;
    }
    let token = match with_state(app, |state| {
        state.app_switch_gen = state.app_switch_gen.wrapping_add(1);
        state.app_switch_gen
    }) {
        Ok(token) => token,
        Err(_) => return,
    };
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        sleep_for(APP_SWITCH_DEBOUNCE).await;
        let current_gen = match with_state(&handle, |state| state.app_switch_gen) {
            Ok(current) => current,
            Err(_) => return,
        };
        if current_gen != token {
            return;
        }
        let Some(now) = workspace::frontmost() else {
            return;
        };
        if now != snapshot || now.1 == handle.config().identifier {
            return;
        }
        if let Err(err) = begin_heckle(handle, Trigger::AppSwitch.as_str().to_string()) {
            eprintln!("app-switch heckle: {err}");
        }
    });
}

async fn sleep_for(duration: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(duration);
    })
    .await;
}

#[derive(Clone, Serialize)]
struct JokePayload {
    text: String,
    font_size: u32,
    popup_opacity: f64,
    chunk_delay_ms: u64,
}

#[derive(Clone, Serialize)]
struct SoundPayload {
    kind: &'static str,
}

#[tauri::command]
fn frontmost_app(app: AppHandle) -> Result<FrontmostApp, String> {
    with_state(&app, |state| FrontmostApp {
        name: state.frontmost_name.clone(),
        bundle_id: state.frontmost_bundle_id.clone(),
    })
}

fn start_wander(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let _ = tauri::async_runtime::spawn_blocking(|| {
                std::thread::sleep(WANDER_TICK);
            })
            .await;
            if let Err(err) = apply_wander_tick(&app) {
                eprintln!("wander: {err}");
            }
        }
    });
}

fn apply_wander_tick(app: &AppHandle) -> Result<(), String> {
    maybe_finish_native_drag(app)?;
    let now = Instant::now();
    if !with_state(app, |state| state.wander_should_move(now))? {
        return Ok(());
    }

    let window = app
        .get_webview_window(OVERLAY_LABEL)
        .ok_or("overlay window missing")?;
    let Some(monitor) = window.current_monitor().map_err(|err| err.to_string())? else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    if scale <= 0.0 {
        return Ok(());
    }
    let work = monitor.work_area();
    let pos = window.outer_position().map_err(|err| err.to_string())?;
    let size = window.outer_size().map_err(|err| err.to_string())?;
    let frame = Frame {
        x: f64::from(work.position.x),
        y: f64::from(work.position.y),
        w: f64::from(work.size.width),
        h: f64::from(work.size.height),
    };

    let (x, y, moved) = with_state(app, |state| {
        state.maybe_repick(now);
        let (x, y, vx, vy) = step_wander(
            f64::from(pos.x),
            f64::from(pos.y),
            f64::from(size.width),
            f64::from(size.height),
            frame,
            state.wander_vx * scale,
            state.wander_vy * scale,
            WANDER_TICK.as_secs_f64(),
        );
        state.wander_vx = vx / scale;
        state.wander_vy = vy / scale;
        let x = x.round() as i32;
        let y = y.round() as i32;
        (x, y, x != pos.x || y != pos.y)
    })?;

    if moved {
        window
            .set_position(PhysicalPosition::new(x, y))
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn maybe_finish_native_drag(app: &AppHandle) -> Result<(), String> {
    let pressed = left_mouse_pressed();
    with_state(app, |state| {
        if !state.dragging {
            return;
        }
        if pressed {
            state.drag_press_seen = true;
            return;
        }
        if state.drag_press_seen {
            state.set_dragging_at(false, Instant::now());
        }
    })
}

fn left_mouse_pressed() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2_app_kit::NSEvent::pressedMouseButtons() & 1 != 0
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[derive(Clone, Copy)]
struct Frame {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn step_wander(
    x: f64,
    y: f64,
    overlay_w: f64,
    overlay_h: f64,
    work: Frame,
    vx: f64,
    vy: f64,
    dt: f64,
) -> (f64, f64, f64, f64) {
    let mut nx = x + vx * dt;
    let mut ny = y + vy * dt;
    let mut nvx = vx;
    let mut nvy = vy;
    let min_x = work.x;
    let min_y = work.y;
    let max_x = (work.x + work.w - overlay_w).max(min_x);
    let max_y = (work.y + work.h - overlay_h).max(min_y);

    if nx < min_x {
        nx = min_x;
        nvx = nvx.abs();
    } else if nx > max_x {
        nx = max_x;
        nvx = -nvx.abs();
    }
    if ny < min_y {
        ny = min_y;
        nvy = nvy.abs();
    } else if ny > max_y {
        ny = max_y;
        nvy = -nvy.abs();
    }
    (nx, ny, nvx, nvy)
}

fn pick_timer_delay(settings: &Settings, rng: &mut Rng) -> Duration {
    let (min, max) = settings.timer_delay_range();
    Duration::from_secs(rng.range_u64(min, max))
}

fn pick_heading(rng: &mut Rng) -> (f64, f64, Duration) {
    let angle = rng.range(0.0, std::f64::consts::TAU);
    let jitter = rng.range(-0.25, 0.25);
    let heading = angle + jitter;
    let speed = rng.range(WANDER_SPEED_MIN, WANDER_SPEED_MAX);
    let interval = Duration::from_secs_f64(rng.range(HEADING_MIN_SECS, HEADING_MAX_SECS));
    (heading.cos() * speed, heading.sin() * speed, interval)
}

struct Rng(u64);

impl Rng {
    fn from_entropy() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self(nanos | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / ((1u64 << 53) as f64))
    }

    fn range(&mut self, min: f64, max: f64) -> f64 {
        min + (max - min) * self.next_f64()
    }

    fn range_u64(&mut self, min: u64, max: u64) -> u64 {
        if min >= max {
            return min;
        }
        let span = max - min + 1;
        min + self.next_u64() % span
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_skips_work_mute_only_changes_pet_state() {
        let mut state = AppState::new(Settings::default());
        assert!(!state.skip_work());
        assert_eq!(state.pet_state(), "idle");
        assert!(!state.wander_paused);

        state.hidden = true;
        state.sync_wander_paused();
        assert!(state.skip_work());
        assert_eq!(state.pet_state(), "idle");
        assert!(state.wander_paused);

        state.muted = true;
        state.sync_wander_paused();
        assert!(state.skip_work());
        assert_eq!(state.pet_state(), "muted");

        state.hidden = false;
        state.sync_wander_paused();
        assert!(!state.skip_work());
        assert_eq!(state.pet_state(), "muted");
        assert!(state.wander_paused);

        state.muted = false;
        state.sync_wander_paused();
        assert_eq!(state.pet_state(), "idle");
        assert!(!state.wander_paused);

        state.joke_showing = true;
        state.sync_wander_paused();
        assert!(state.wander_paused);
        assert!(!state.wander_should_move(Instant::now()));
    }

    #[test]
    fn wander_step_stays_inside_visible_frame_and_reflects() {
        let work = Frame {
            x: 10.0,
            y: 25.0,
            w: 800.0,
            h: 500.0,
        };
        let (x, y, vx, vy) = step_wander(10.0, 25.0, 160.0, 160.0, work, -40.0, -40.0, 0.05);
        assert_eq!(x, 10.0);
        assert_eq!(y, 25.0);
        assert!(vx > 0.0);
        assert!(vy > 0.0);

        let (x, y, vx, vy) = step_wander(650.0, 365.0, 160.0, 160.0, work, 40.0, 40.0, 0.05);
        assert_eq!(x, 650.0);
        assert_eq!(y, 365.0);
        assert!(vx < 0.0);
        assert!(vy < 0.0);
    }

    #[test]
    fn wander_step_moves_by_velocity_and_dt() {
        let work = Frame {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 1000.0,
        };
        let (x, y, vx, vy) = step_wander(100.0, 200.0, 160.0, 160.0, work, 20.0, 0.0, 0.05);
        assert_eq!(x, 101.0);
        assert_eq!(y, 200.0);
        assert_eq!(vx, 20.0);
        assert_eq!(vy, 0.0);
    }

    #[test]
    fn timer_delay_stays_in_settings_range() {
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        let ranged = Settings {
            timer_min_seconds: 12,
            timer_max_seconds: 18,
            ..Settings::default()
        };
        for _ in 0..80 {
            let delay = pick_timer_delay(&ranged, &mut rng);
            assert!((12..=18).contains(&delay.as_secs()));
        }

        let fixed = Settings {
            timer_min_seconds: 45,
            timer_max_seconds: 45,
            ..Settings::default()
        };
        assert_eq!(pick_timer_delay(&fixed, &mut rng), Duration::from_secs(45));

        let inverted = Settings {
            timer_min_seconds: 80,
            timer_max_seconds: 20,
            ..Settings::default()
        };
        for _ in 0..40 {
            let delay = pick_timer_delay(&inverted, &mut rng);
            assert!((20..=80).contains(&delay.as_secs()));
        }
    }

    #[test]
    fn heading_speed_and_interval_stay_in_range() {
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        for _ in 0..40 {
            let (vx, vy, interval) = pick_heading(&mut rng);
            let speed = (vx * vx + vy * vy).sqrt();
            assert!(speed >= WANDER_SPEED_MIN && speed <= WANDER_SPEED_MAX);
            assert!(interval.as_secs_f64() >= HEADING_MIN_SECS);
            assert!(interval.as_secs_f64() <= HEADING_MAX_SECS);
        }
    }

    #[test]
    fn mute_hide_drag_and_joke_pause_wander() {
        let mut state = AppState::new(Settings::default());
        let t0 = Instant::now();
        assert!(state.wander_should_move(t0));

        state.set_dragging_at(true, t0);
        assert!(!state.wander_should_move(t0));

        state.set_dragging_at(false, t0);
        assert!(!state.wander_should_move(t0 + Duration::from_secs(2)));
        assert!(state.wander_should_move(t0 + DRAG_RESUME));

        state.muted = true;
        state.sync_wander_paused();
        assert!(!state.wander_should_move(t0 + Duration::from_secs(10)));

        state.muted = false;
        state.sync_wander_paused();
        state.hidden = true;
        state.sync_wander_paused();
        assert!(!state.wander_should_move(t0 + Duration::from_secs(10)));
    }

    #[test]
    fn lurking_and_talking_beat_muted_in_pet_state() {
        let mut state = AppState::new(Settings::default());
        state.muted = true;
        state.phase = HecklePhase::Lurking;
        assert_eq!(state.pet_state(), "lurking");
        state.phase = HecklePhase::Talking;
        assert_eq!(state.pet_state(), "talking");
        state.phase = HecklePhase::Idle;
        assert_eq!(state.pet_state(), "muted");
    }

    #[test]
    fn heckle_guards_match_trigger_rules() {
        let mut state = AppState::new(Settings::default());
        let t0 = Instant::now();
        let cooldown = Duration::from_secs(60);

        assert!(!heckle_blocked(&state, Trigger::Click, t0, cooldown));
        assert!(!heckle_blocked(&state, Trigger::Timer, t0, cooldown));
        assert!(!heckle_blocked(&state, Trigger::AppSwitch, t0, cooldown));

        state.hidden = true;
        assert!(heckle_blocked(&state, Trigger::Click, t0, cooldown));
        assert!(heckle_blocked(&state, Trigger::Timer, t0, cooldown));
        state.hidden = false;

        state.in_flight = true;
        assert!(heckle_blocked(&state, Trigger::Click, t0, cooldown));
        assert!(heckle_blocked(&state, Trigger::AppSwitch, t0, cooldown));
        state.in_flight = false;

        state.muted = true;
        assert!(!heckle_blocked(&state, Trigger::Click, t0, cooldown));
        assert!(heckle_blocked(&state, Trigger::Timer, t0, cooldown));
        assert!(heckle_blocked(&state, Trigger::AppSwitch, t0, cooldown));
        state.muted = false;

        state.joke_showing = true;
        assert!(!heckle_blocked(&state, Trigger::Click, t0, cooldown));
        assert!(heckle_blocked(&state, Trigger::Timer, t0, cooldown));
        state.joke_showing = false;

        state.last_click_at = Some(t0);
        assert!(heckle_blocked(
            &state,
            Trigger::Click,
            t0 + Duration::from_secs(9),
            cooldown
        ));
        assert!(!heckle_blocked(
            &state,
            Trigger::Click,
            t0 + CLICK_COOLDOWN,
            cooldown
        ));

        state.last_joke_at = Some(t0);
        assert!(heckle_blocked(
            &state,
            Trigger::Timer,
            t0 + Duration::from_secs(59),
            cooldown
        ));
        assert!(heckle_blocked(
            &state,
            Trigger::AppSwitch,
            t0 + Duration::from_secs(59),
            cooldown
        ));
        assert!(!heckle_blocked(
            &state,
            Trigger::Click,
            t0 + CLICK_COOLDOWN,
            cooldown
        ));
        assert!(!heckle_blocked(
            &state,
            Trigger::Timer,
            t0 + cooldown,
            cooldown
        ));
    }

    #[test]
    fn joke_window_grows_upward_and_shrinks_back() {
        let (x, y) = resize_origin(100.0, 200.0, 160.0, 160.0, 320.0, 320.0);
        assert_eq!(x, 20.0);
        assert_eq!(y, 40.0);
        let (x, y) = resize_origin(x, y, 320.0, 320.0, 160.0, 160.0);
        assert_eq!(x, 100.0);
        assert_eq!(y, 200.0);
    }

    #[test]
    fn sfx_allowed_honors_sound_mute_and_hide() {
        assert!(sfx_allowed(true, false, false));
        assert!(!sfx_allowed(true, true, false));
        assert!(!sfx_allowed(false, false, false));
        assert!(!sfx_allowed(true, false, true));
        assert!(!sfx_allowed(false, true, false));
    }

    #[test]
    fn joke_display_limit_covers_chunk_pages() {
        assert_eq!(
            joke_display_limit(Duration::from_secs(8)),
            Duration::from_secs(512)
        );
        assert_eq!(
            joke_display_limit(Duration::from_secs(3)),
            Duration::from_secs(192)
        );
        assert_eq!(joke_display_limit(Duration::ZERO), BUBBLE * CHUNK_LIMIT);
    }

    #[test]
    fn joke_window_origin_stays_inside_visible_frame() {
        let work = Frame {
            x: 0.0,
            y: 25.0,
            w: 300.0,
            h: 240.0,
        };
        let (x, y) = resize_origin(0.0, 25.0, 160.0, 160.0, 320.0, 320.0);
        let (x, y) = clamp_origin(x, y, 320.0, 320.0, work);
        assert_eq!(x, 0.0);
        assert_eq!(y, 25.0);
    }
}
