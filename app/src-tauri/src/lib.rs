//! Tauri shell: builds the identity service on the OS boundaries and exposes
//! the identity commands to the webview.

mod identity_cmd;

use messenger_core::identity::{
    IdentityService, MemorySettingsStore, NodeIdTicketProvider, OsKeychain,
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
            let service: AppIdentity = IdentityService::new(
                OsKeychain::native()?,
                MemorySettingsStore::new(),
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
