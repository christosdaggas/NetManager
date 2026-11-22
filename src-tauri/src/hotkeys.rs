use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use netmanager_core::models::HotkeyEntry;
use tauri::{AppHandle, Emitter};
use tokio::sync::watch;

type Bindings = Vec<(String, String, String, String)>;
pub struct Hotkeys(watch::Sender<Bindings>);
impl Hotkeys {
    pub fn start(app: AppHandle) -> Self {
        let (tx, mut rx) = watch::channel(Bindings::new());
        tauri::async_runtime::spawn(async move {
            while rx.changed().await.is_ok() {
                let entries = rx.borrow_and_update().clone();
                if entries.is_empty() {
                    continue;
                }
                if let Err(error) = listen(&app, &entries, &mut rx).await {
                    let _=app.emit_to("main","service-message",format!("Desktop shortcuts unavailable: {error}. Shortcuts still work inside the app."));
                }
            }
        });
        Self(tx)
    }
    pub fn update(&self, entries: &[HotkeyEntry], enabled: bool) {
        let bindings: Bindings = entries
            .iter()
            .filter(|e| enabled && e.enabled)
            .map(|e| {
                let mut parts: Vec<_> = e
                    .modifiers
                    .iter()
                    .map(|s| match s.as_str() {
                        "Ctrl" => "CTRL",
                        "Alt" => "ALT",
                        "Shift" => "SHIFT",
                        "Super" => "LOGO",
                        other => other,
                    })
                    .collect();
                parts.push(&e.key);
                (
                    e.id.clone(),
                    e.profile_id.clone(),
                    e.profile_name.clone(),
                    parts.join("+"),
                )
            })
            .collect();
        self.0.send_if_modified(|current| {
            if *current != bindings {
                *current = bindings;
                true
            } else {
                false
            }
        });
    }
}
async fn listen(
    app: &AppHandle,
    entries: &Bindings,
    rx: &mut watch::Receiver<Bindings>,
) -> Result<(), ashpd::Error> {
    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session(Default::default()).await?;
    let shortcuts: Vec<_> = entries
        .iter()
        .map(|(id, _, name, trigger)| {
            NewShortcut::new(id.as_str(), format!("Switch to {name}"))
                .preferred_trigger(trigger.as_str())
        })
        .collect();
    let active = portal.receive_activated().await?;
    let path = serde_json::to_value(&session).ok();
    let result = async {
        portal
            .bind_shortcuts(&session, &shortcuts, None, Default::default())
            .await?
            .response()?;
        futures_util::pin_mut!(active);
        while let Some(e) = active.next().await {
            if path.as_ref().and_then(|v| v.as_str()) != Some(e.session_handle().as_str()) {
                continue;
            }
            if let Some((_, profile, _, _)) = entries.iter().find(|(id, ..)| id == e.shortcut_id())
            {
                let _ = app.emit_to("main", "profile-request", (profile, false));
            }
        }
        Ok(())
    };
    // Observe without advancing the outer receiver: the next iteration must bind the new set.
    let mut changes = rx.clone();
    let r = tokio::select! {r=result=>r,_=changes.changed()=>Ok(())};
    session.close().await?;
    r
}
