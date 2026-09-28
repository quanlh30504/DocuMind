mod core;

use crate::core::document::{self, DocumentSource};
use crate::core::hardware::{HardwareManager, HardwareProfile};
use crate::core::traits::{DependencyManager, InstallState};
use crate::core::DependencyManagerImpl;

#[tauri::command]
fn get_hardware_profile() -> HardwareProfile {
    HardwareManager::profile()
}

#[tauri::command]
fn discover_documents(root: String) -> Result<Vec<DocumentSource>, String> {
    document::discover(std::path::Path::new(&root)).map_err(|e| e.to_string())
}

/// Reports whether the bundled English OCR runtime/model set is present, so the
/// UI can show "OCR ready" / "OCR missing" per spec §2 ("must not silently
/// continue without OCR"). Full install/repair flow lands in Phase 2.
#[tauri::command]
fn ocr_runtime_status() -> String {
    let manager = DependencyManagerImpl;
    let id = crate::core::traits::RuntimeId("onnxruntime".to_string());
    match manager.check_runtime(&id) {
        InstallState::Installed => "installed".to_string(),
        InstallState::Missing => "missing".to_string(),
        InstallState::Corrupt => "corrupt".to_string(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_hardware_profile,
            discover_documents,
            ocr_runtime_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
