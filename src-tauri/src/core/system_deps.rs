//! Checks and (on Linux) installs the **system CLI tools** Phase 2's OCR
//! pipeline currently shells out to (`tesseract`, `pdftoppm`/`pdftotext`,
//! and the Hunspell dictionaries `core/spellcheck.rs` reads directly off
//! disk).
//!
//! This is deliberately a separate, narrower thing from
//! `core/dependency.rs`'s `DependencyManager` trait, which models
//! DEPENDENCY_STRATEGY.md's future bundled/downloaded-binary story (ONNX
//! Runtime, llama.cpp — checksummed archives the app manages itself). What's
//! here instead drives the host OS's own package manager, because that's
//! what today's implementation actually depends on — conflating the two
//! would overstate what's real. Once RapidOCR-ONNX + `ort` lands (replacing
//! the system Tesseract dependency, per `MVP_PLAN.md`'s Phase 2
//! engine-sequencing note), this module's job shrinks to just Poppler/
//! spellcheck dictionaries or disappears entirely.

use crate::core::ocr::TesseractOcrProvider;
use crate::core::pdf;
use std::process::Command;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SystemDependency {
    /// Human-readable name for the UI.
    pub name: String,
    /// apt package that provides it — used both for display and as the
    /// literal argument to `apt-get install`.
    pub package: &'static str,
    pub installed: bool,
}

fn hunspell_dict_exists(base: &str) -> bool {
    std::path::Path::new(&format!("/usr/share/hunspell/{base}.aff")).exists()
        && std::path::Path::new(&format!("/usr/share/hunspell/{base}.dic")).exists()
}

/// The full set Phase 2 needs for its default "auto" (`eng+vie`) language
/// selection. Checking a fixed list rather than deriving it from user
/// selection keeps this simple and matches what the app bundles/ships as
/// its default OCR+spellcheck language pair (`MODEL_STRATEGY.md` §1).
pub fn check_all() -> Vec<SystemDependency> {
    let installed_langs = TesseractOcrProvider::installed_langs();
    vec![
        SystemDependency {
            name: "Tesseract OCR engine".into(),
            package: "tesseract-ocr",
            installed: TesseractOcrProvider::is_available(),
        },
        SystemDependency {
            name: "Tesseract English language data".into(),
            package: "tesseract-ocr-eng",
            installed: installed_langs.iter().any(|l| l == "eng"),
        },
        SystemDependency {
            name: "Tesseract Vietnamese language data".into(),
            package: "tesseract-ocr-vie",
            installed: installed_langs.iter().any(|l| l == "vie"),
        },
        SystemDependency {
            name: "PDF tools (poppler-utils)".into(),
            package: "poppler-utils",
            installed: pdf::require_poppler().is_ok(),
        },
        SystemDependency {
            name: "English spelling dictionary".into(),
            package: "hunspell-en-us",
            installed: hunspell_dict_exists("en_US"),
        },
        SystemDependency {
            name: "Vietnamese spelling dictionary".into(),
            package: "hunspell-vi",
            installed: hunspell_dict_exists("vi_VN"),
        },
    ]
}

pub fn missing_packages(deps: &[SystemDependency]) -> Vec<&'static str> {
    deps.iter().filter(|d| !d.installed).map(|d| d.package).collect()
}

fn has_apt() -> bool {
    Command::new("apt-get").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn has_pkexec() -> bool {
    Command::new("pkexec").arg("--version").output().is_ok_and(|o| o.status.success())
}

/// The exact command a user would run themselves — shown whenever automatic
/// install isn't available or fails, so the fallback is never "figure it
/// out yourself" (spec §2: explain the problem, don't silently give up).
pub fn manual_install_command(packages: &[&str]) -> String {
    format!("sudo apt-get install -y {}", packages.join(" "))
}

/// Installs `packages` via `pkexec apt-get install -y ...` — a graphical
/// polkit prompt for the password, appropriate for a GUI app (no terminal to
/// type a `sudo` password into). Blocking; call from a background thread.
/// Only implemented for apt-based Linux today (this app's primary target
/// per `PACKAGING.md`); other platforms/package managers get a clear "not
/// automatable here, run this yourself" error rather than a silent no-op.
pub fn install_missing(packages: &[&str]) -> Result<String, String> {
    if packages.is_empty() {
        return Ok("Nothing to install.".to_string());
    }
    if cfg!(not(target_os = "linux")) {
        return Err(format!(
            "Automatic install isn't available on this OS yet. Install manually: {}",
            manual_install_command(packages)
        ));
    }
    if !has_apt() {
        return Err(format!(
            "Automatic install currently only supports apt-based Linux. Install manually: {}",
            manual_install_command(packages)
        ));
    }
    if !has_pkexec() {
        return Err(format!(
            "pkexec (needed to ask for your password graphically) isn't available. Install manually in a terminal: {}",
            manual_install_command(packages)
        ));
    }

    let output = Command::new("pkexec")
        .arg("apt-get")
        .arg("install")
        .arg("-y")
        .args(packages)
        .output()
        .map_err(|e| format!("failed to run pkexec: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Exit code 126/127 from pkexec means the user cancelled the
        // password prompt — distinguish that from a real apt failure so the
        // UI doesn't call a user-cancelled dialog an "error".
        if output.status.code() == Some(126) || output.status.code() == Some(127) {
            Err("Cancelled — no changes made.".to_string())
        } else {
            Err(format!(
                "apt-get install failed: {stderr}\n\nYou can also run this yourself: {}",
                manual_install_command(packages)
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_packages_lists_only_uninstalled() {
        let deps = vec![
            SystemDependency { name: "a".into(), package: "pkg-a", installed: true },
            SystemDependency { name: "b".into(), package: "pkg-b", installed: false },
        ];
        assert_eq!(missing_packages(&deps), vec!["pkg-b"]);
    }

    #[test]
    fn manual_install_command_is_copy_pasteable() {
        assert_eq!(manual_install_command(&["a", "b"]), "sudo apt-get install -y a b");
    }
}
