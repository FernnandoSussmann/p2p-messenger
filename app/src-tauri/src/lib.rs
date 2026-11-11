//! Tauri shell: builds the identity service on the OS boundaries, runs the
//! network node and exposes the identity and network commands to the webview.

mod contacts_cmd;
mod identity_cmd;
mod net_cmd;
mod profile;
#[cfg(test)]
mod test_util;

use std::sync::Arc;
use std::time::Duration;

use messenger_core::contacts::{migrate_legacy_known_peers, FileContactStore, NoMessageStore};
use messenger_core::discovery::LiveTicketProvider;
use messenger_core::identity::{FileSettingsStore, IdentityService, IdentityStatus, OsKeychain};
use tauri::{Manager, RunEvent};

const APP_TITLE: &str = "p2p-messenger";

use contacts_cmd::ContactsState;
use identity_cmd::{
    identity_my_info, identity_provision, identity_set_display_name, identity_status, AppIdentity,
};
use net_cmd::{
    ensure_started, net_connect_ticket, net_lan_peers, net_my_ticket, net_pending_incoming,
    net_presence_get, net_presence_set, net_respond_incoming, relay_override_from_env, NetBoot,
    NetState,
};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let profile = profile::profile_from(
                std::env::args().skip(1),
                std::env::var("MESSENGER_PROFILE").ok(),
            )
            .map_err(|e| format!("invalid profile: {e}"))?;
            let relay = relay_override_from_env(std::env::var("MESSENGER_RELAY_URL").ok())?;
            let layout = profile::layout(&app.path().app_data_dir()?, profile);
            let data_dir = layout.data_dir;
            std::fs::create_dir_all(&data_dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| format!("cannot restrict app data directory permissions: {e}"))?;
            }
            let contacts_store = Arc::new(FileContactStore::new(data_dir.clone()));
            // The app crate has no tracing dependency; counts only, never names.
            match migrate_legacy_known_peers(&data_dir, contacts_store.as_ref()) {
                Ok(r) if r.migrated > 0 => {
                    eprintln!("migrated {} known peers to contacts", r.migrated);
                }
                Ok(_) => {}
                Err(_) => {
                    eprintln!("legacy known peers not migrated; those peers will be prompted again")
                }
            }
            app.manage(Arc::new(ContactsState::new(
                contacts_store,
                Arc::new(NoMessageStore),
            )));
            let settings = Arc::new(FileSettingsStore::new(data_dir.clone()));
            let tickets = LiveTicketProvider::new();
            let service: AppIdentity = IdentityService::new(
                OsKeychain::native_for(layout.profile.as_ref())?,
                Arc::clone(&settings),
                tickets.clone(),
                data_dir.clone(),
            );
            let present = matches!(service.status(), IdentityStatus::Present { .. });
            app.manage(service);
            if let Some(window) = app.get_webview_window("main") {
                window.set_title(&profile::window_title(APP_TITLE, layout.profile.as_ref()))?;
            }
            app.manage(NetState::new(relay));
            app.manage(NetBoot { settings, tickets });
            if present {
                ensure_started(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            identity_status,
            identity_provision,
            identity_set_display_name,
            identity_my_info,
            net_my_ticket,
            net_connect_ticket,
            net_respond_incoming,
            net_pending_incoming,
            net_presence_get,
            net_presence_set,
            net_lan_peers
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
            let state = app.state::<NetState>();
            tauri::async_runtime::block_on(state.shutdown_with_timeout(SHUTDOWN_TIMEOUT));
        }
    });
}
