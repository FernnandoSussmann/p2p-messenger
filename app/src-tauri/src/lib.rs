//! Tauri shell: builds the identity service on the OS boundaries, runs the
//! network node and exposes the identity and network commands to the webview.

mod identity_cmd;
mod net_cmd;
mod profile;
#[cfg(test)]
mod test_util;

use std::sync::Arc;
use std::time::Duration;

use messenger_core::discovery::LiveTicketProvider;
use messenger_core::identity::{FileSettingsStore, IdentityService, IdentityStatus, OsKeychain};
use tauri::{Manager, RunEvent};

use identity_cmd::{
    identity_my_info, identity_provision, identity_set_display_name, identity_status, AppIdentity,
};
use net_cmd::{
    ensure_started, net_connect_ticket, net_my_ticket, net_pending_incoming, net_respond_incoming,
    NetBoot, NetState,
};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| format!("cannot restrict app data directory permissions: {e}"))?;
            }
            let settings = Arc::new(FileSettingsStore::new(data_dir.clone()));
            let tickets = LiveTicketProvider::new();
            let service: AppIdentity = IdentityService::new(
                OsKeychain::native()?,
                Arc::clone(&settings),
                tickets.clone(),
                data_dir.clone(),
            );
            let present = matches!(service.status(), IdentityStatus::Present { .. });
            app.manage(service);
            app.manage(NetState::default());
            app.manage(NetBoot {
                data_dir,
                settings,
                tickets,
            });
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
            net_pending_incoming
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
