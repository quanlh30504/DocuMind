pub mod core;

use crate::core::document::{self, DocumentSource};
use crate::core::hardware::{HardwareManager, HardwareProfile};
use crate::core::job::{self, JobEvent, JobState};
use crate::core::ocr::TesseractOcrProvider;
use tauri::{AppHandle, Emitter};

#[tauri::command]
fn get_hardware_profile() -> HardwareProfile {
    HardwareManager::profile()
}

#[tauri::command]
fn discover_documents(root: String) -> Result<Vec<DocumentSource>, String> {
    document::discover(std::path::Path::new(&root)).map_err(|e| e.to_string())
}

/// Reports whether OCR is usable right now, so the UI can show "OCR ready" /
/// "OCR missing" per spec §2 ("must not silently continue without OCR").
/// Phase 2 uses the system Tesseract binary (see core/ocr/tesseract.rs); this
/// will grow to also check the bundled ONNX runtime once that lands.
#[tauri::command]
fn ocr_runtime_status() -> String {
    if TesseractOcrProvider::is_available() {
        "installed".to_string()
    } else {
        "missing".to_string()
    }
}

/// Runs one document through the pipeline (ARCHITECTURE.md §3) and writes
/// `output_dir/{raw,metadata.json,full_document.md}` (spec §24). Emits
/// `job://progress` events as pages complete so the UI can show a progress
/// bar; a single page failing does not fail the whole call (spec §45) — it's
/// recorded in the returned `JobState` instead.
#[tauri::command]
async fn process_document(
    app: AppHandle,
    source: DocumentSource,
    output_dir: String,
) -> Result<JobState, String> {
    let output_dir = std::path::PathBuf::from(output_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let ocr = TesseractOcrProvider::default();
        job::run_document(&source, &output_dir, &ocr, |event: JobEvent| {
            let _ = app.emit("job://progress", &event);
        })
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_hardware_profile,
            discover_documents,
            ocr_runtime_status,
            process_document
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
