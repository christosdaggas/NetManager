use chrono::{Local, Utc};
use netmanager_core::{
    autoswitch::AutoSwitchService,
    models::{AppConfig, Profile, WatchdogAction},
    scheduler::SchedulerService,
    storage::DataStore,
    tray::{self, TrayCommand},
    watchdog::WatchdogService,
};
use std::{
    sync::{mpsc, OnceLock},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

pub struct Services(mpsc::Sender<(AppConfig, Vec<Profile>)>);
impl Services {
    pub fn update(&self, cfg: &AppConfig, profiles: &[Profile]) {
        let mut cfg = cfg.clone();
        cfg.encryption_key = None;
        cfg.encryption_check = None;
        let mut list = profiles.to_vec();
        for p in &mut list {
            p.network_actions.clear();
            p.system_actions.clear();
            p.automation_actions.clear();
        }
        let _ = self.0.send((cfg, list));
    }
    pub fn start(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || worker(app, rx));
        Self(tx)
    }
}
fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
fn activate(app: &AppHandle, id: &str, automatic: bool) {
    let _ = app.emit_to("main", "profile-request", (id, automatic));
}
fn worker(app: AppHandle, rx: mpsc::Receiver<(AppConfig, Vec<Profile>)>) {
    let Ok((mut cfg, mut profiles)) = rx.recv() else {
        return;
    };
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let rt = RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .expect("tray runtime")
    });
    let mut tray = None;
    let mut requested = false;
    let mut first = true;
    let mut changed = true;
    let mut minute = None;
    let mut switch = AutoSwitchService::new();
    let mut last_auto = Instant::now();
    let mut dog = WatchdogService::new(cfg.watchdog.clone());
    let mut last_ping = Instant::now();
    loop {
        if requested != cfg.show_tray_icon {
            if cfg.show_tray_icon {
                tray = tray::start_tray(rt);
            } else if let Some((h, _)) = tray.take() {
                rt.block_on(h.shutdown());
            }
            requested = cfg.show_tray_icon;
            changed = true;
        }
        if let Some((h, commands)) = &tray {
            if first
                && (cfg.start_minimized
                    || std::env::args().any(|a| a == "--minimized" || a == "-m"))
            {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            if changed {
                h.set_profiles(
                    profiles
                        .iter()
                        .map(|p| (p.id().to_string(), p.name().to_owned()))
                        .collect(),
                );
                h.set_active_profile(
                    profiles
                        .iter()
                        .filter(|p| p.metadata.last_applied_at.is_some())
                        .max_by_key(|p| p.metadata.last_applied_at)
                        .map(|p| p.id().to_string()),
                );
            }
            for command in commands.try_iter() {
                match command {
                    TrayCommand::Quit => app.exit(0),
                    TrayCommand::ShowWindow => show(&app),
                    TrayCommand::ActivateProfile(id) => {
                        show(&app);
                        activate(&app, &id, false)
                    }
                }
            }
        }
        first = false;
        changed = false;
        let (times, last) = SchedulerService::minutes_to_evaluate(minute, Utc::now());
        minute = Some(last);
        if cfg.scheduling_enabled && !profiles.is_empty() {
            let times: Vec<_> = times.iter().map(|t| t.with_timezone(&Local)).collect();
            let due = SchedulerService::due_schedules(&cfg.schedules, &times);
            let mut once = vec![];
            for entry in due {
                if profiles
                    .iter()
                    .any(|p| p.id().to_string() == entry.profile_id)
                {
                    activate(&app, &entry.profile_id, false);
                    if entry.one_shot {
                        once.push(entry.id.clone());
                    }
                }
            }
            if !once.is_empty() {
                SchedulerService::disable_one_shots(&mut cfg.schedules, &once);
                let lib = app.state::<crate::library::Library>();
                if let Ok(_lock) = lib.guard() {
                    let s = DataStore::new();
                    if let Ok(mut live) = crate::library::config(&s, None) {
                        SchedulerService::disable_one_shots(&mut live.schedules, &once);
                        let _ = s.save_config(&live);
                    }
                };
            }
        }
        if cfg.auto_switch_enabled
            && last_auto.elapsed().as_secs() >= u64::from(cfg.auto_switch_interval_secs.max(5))
        {
            last_auto = Instant::now();
            if let Some(id) = switch.evaluate_profiles(&profiles) {
                activate(&app, &id, true);
            }
        }
        if cfg.watchdog.enabled
            && last_ping.elapsed().as_secs() >= u64::from(cfg.watchdog.check_interval_secs.max(5))
        {
            last_ping = Instant::now();
            if let Some(a) = dog.check() {
                if let Err(e) = dog.execute_action(a) {
                    let _ = app.emit_to("main", "service-message", e);
                }
                match a {
                    WatchdogAction::SwitchProfile => {
                        if let Some(id) = dog.fallback_profile_id() {
                            activate(&app, id, false);
                        }
                    }
                    WatchdogAction::Notify => {
                        let _ = app.emit_to(
                            "main",
                            "service-message",
                            "Connection lost. Check your network.",
                        );
                    }
                    _ => {}
                }
            }
        }
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok((next, list)) => {
                if next.watchdog != cfg.watchdog {
                    dog = WatchdogService::new(next.watchdog.clone());
                    last_ping = Instant::now();
                }
                cfg = next;
                profiles = list;
                changed = true;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}
