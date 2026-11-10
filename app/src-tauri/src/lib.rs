//! Tauri shell: builds the identity service on the OS boundaries and exposes
//! the identity commands to the webview.

mod identity_cmd;

use messenger_core::identity::{
    FileSettingsStore, IdentityService, NodeIdTicketProvider, OsKeychain,
};
use tauri::Manager;

use identity_cmd::{
    identity_my_info, identity_provision, identity_set_display_name, identity_status, AppIdentity,
};

pub fn run() {
    tauri::Builder::default()
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
            let service: AppIdentity = IdentityService::new(
                OsKeychain::native()?,
                FileSettingsStore::new(data_dir.clone()),
                NodeIdTicketProvider,
                data_dir,
            );
            app.manage(service);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            identity_status,
            identity_provision,
            identity_set_display_name,
            identity_my_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
