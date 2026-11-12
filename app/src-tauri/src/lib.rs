//! Tauri shell: builds the identity service on the OS boundaries, runs the
//! network node and exposes the identity and network commands to the webview.

mod chat_cmd;
mod contacts_cmd;
mod identity_cmd;
mod net_cmd;
mod profile;
mod storage_cmd;
#[cfg(test)]
mod test_util;

use std::sync::Arc;
use std::time::Duration;

use messenger_core::contacts::SqlCipherContactStore;
use messenger_core::discovery::LiveTicketProvider;
use messenger_core::identity::{
    IdentityService, IdentityStatus, OsKeychain, SqlCipherSettingsStore,
};
use messenger_core::logging::{self, LogConfig};
use messenger_core::storage::Storage;
use tauri::{Manager, RunEvent};

const APP_TITLE: &str = "p2p-messenger";

use chat_cmd::{chat_history, chat_resend, chat_send, history_clear, history_search, ChatState};
use contacts_cmd::{
    contacts_add, contacts_is_known, contacts_list, contacts_remove, contacts_rename,
    contacts_set_verified, ContactsState,
};
use identity_cmd::{
    identity_my_info, identity_provision, identity_set_display_name, identity_status, AppIdentity,
};
use net_cmd::{
    ensure_started, net_connect_ticket, net_lan_peers, net_my_ticket, net_pending_incoming,
    net_presence_get, net_presence_set, net_respond_incoming, relay_override_from_env, NetBoot,
    NetState,
};
use storage_cmd::{storage_retry, storage_status};

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
            let layout =
                profile::layout(&app.path().app_data_dir()?, &app.path().app_log_dir()?, profile);
            let level = logging::level_from_env(std::env::var("MESSENGER_LOG").ok().as_deref())?;
            if logging::init(&LogConfig::new(layout.log_dir, level)).is_err() {
                let _ = std::io::Write::write_all(
                    &mut std::io::stderr(),
                    b"p2p-messenger: diagnostic log file unavailable; continuing without file logging\n",
                );
            }
            let data_dir = layout.data_dir;
            std::fs::create_dir_all(&data_dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| format!("cannot restrict app data directory permissions: {e}"))?;
            }
            let keychain = Arc::new(OsKeychain::native_for(layout.profile.as_ref())?);
            let storage = Arc::new(Storage::new(data_dir.clone(), keychain.clone()));
            let contacts_store = Arc::new(SqlCipherContactStore::new(storage.clone()));
            let chat_state = Arc::new(ChatState::new());
            app.manage(Arc::new(ContactsState::new(
                contacts_store,
                chat_state.clone(),
            )));
            app.manage(chat_state.clone());
            let settings = Arc::new(SqlCipherSettingsStore::new(storage.clone()));
            let tickets = LiveTicketProvider::new();
            let service: AppIdentity = IdentityService::new(
                keychain,
                Arc::clone(&settings),
                tickets.clone(),
                data_dir.clone(),
            );
            // Open eagerly when an identity exists, so the first `storage_status` is
            // already decided; a failure is latched and shown by the UI, never
            // auto-retried.
            let present = match service.status() {
                IdentityStatus::Present { account_id, .. } => {
                    let _ = storage_cmd::open_for(&storage, &chat_state, &account_id);
                    true
                }
                _ => false,
            };
            app.manage(storage);
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
            net_lan_peers,
            contacts_list,
            contacts_add,
            contacts_rename,
            contacts_set_verified,
            contacts_remove,
            contacts_is_known,
            chat_send,
            chat_resend,
            chat_history,
            history_search,
            history_clear,
            storage_status,
            storage_retry
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
            app.state::<Arc<ChatState>>().shutdown();
            let state = app.state::<NetState>();
            tauri::async_runtime::block_on(state.shutdown_with_timeout(SHUTDOWN_TIMEOUT));
        }
    });
}
