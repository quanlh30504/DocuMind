pub mod core;

use crate::core::document::{self, DocumentSource};
use crate::core::hardware::{HardwareManager, HardwareProfile};
use crate::core::job::{self, JobEvent, JobState};
use crate::core::ocr::TesseractOcrProvider;
use crate::core::spellcheck::SpellChecker;
use crate::core::system_deps::{self, SystemDependency};
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

/// Language codes with tessdata actually installed (spec §30's "Models"
/// settings list), so the UI only offers languages that will really work.
#[tauri::command]
fn ocr_languages() -> Vec<String> {
    TesseractOcrProvider::installed_langs()
}

/// Runs one document through the pipeline (ARCHITECTURE.md §3) and writes
/// `output_dir/{raw,metadata.json,full_document.md}` (spec §24). Emits
/// `job://progress` events as pages complete so the UI can show a progress
/// bar; a single page failing does not fail the whole call (spec §45) — it's
/// recorded in the returned `JobState` instead.
///
/// `lang` is `None`/`"auto"` (load every installed language together) or an
/// explicit code (`"eng"`, `"vie"`, ...) from `ocr_languages`.
#[tauri::command]
async fn process_document(
    app: AppHandle,
    source: DocumentSource,
    output_dir: String,
    lang: Option<String>,
) -> Result<JobState, String> {
    let output_dir = std::path::PathBuf::from(output_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let resolved_lang = TesseractOcrProvider::resolve_lang(lang.as_deref());
        let ocr = TesseractOcrProvider::new(resolved_lang.clone());
        let spell_checker = SpellChecker::load(&resolved_lang);
        job::run_document(
            &source,
            &output_dir,
            &ocr,
            Some(&spell_checker),
            |event: JobEvent| {
                let _ = app.emit("job://progress", &event);
            },
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Checks the system CLI tools Phase 2's OCR pipeline depends on
/// (core/system_deps.rs) — Tesseract + language packs, Poppler, spellcheck
/// dictionaries — so the UI can show exactly what's missing rather than a
/// generic "OCR not ready" (spec §2).
#[tauri::command]
fn check_system_dependencies() -> Vec<SystemDependency> {
    system_deps::check_all()
}

/// Installs whatever `check_system_dependencies` reports missing, via
/// `pkexec apt-get install` (graphical password prompt) on apt-based Linux.
/// Blocking (waits on the user's password prompt / apt), hence
/// `spawn_blocking`. On any platform/setup where that's not possible,
/// returns the manual command instead of silently doing nothing.
#[tauri::command]
async fn install_system_dependencies() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let deps = system_deps::check_all();
        let missing = system_deps::missing_packages(&deps);
        system_deps::install_missing(&missing)
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
            ocr_languages,
            process_document,
            check_system_dependencies,
            install_system_dependencies
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
