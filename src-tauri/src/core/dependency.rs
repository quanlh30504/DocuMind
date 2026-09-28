//! DependencyManager (spec §40, DEPENDENCY_STRATEGY.md §6).
//!
//! Phase 1 scope: platform detection and on-disk presence checks only. The
//! download/verify/install pipeline (DEPENDENCY_STRATEGY.md §4-5) is Phase 2
//! work — this file exists now so the trait boundary is exercised end-to-end
//! (UI -> Tauri command -> DependencyManager) before that pipeline is built.

use crate::core::hardware::PlatformTarget;
use crate::core::traits::{CoreError, CoreResult, DependencyManager, InstallState, RuntimeId};

pub struct DependencyManagerImpl;

impl DependencyManagerImpl {
    fn runtime_dir(&self, id: &RuntimeId) -> std::path::PathBuf {
        crate::core::document::app_data_dir()
            .join("runtime")
            .join(&id.0)
            .join(format!("{:?}", PlatformTarget::detect()).to_lowercase())
    }
}

impl DependencyManager for DependencyManagerImpl {
    fn detect_platform(&self) -> PlatformTarget {
        PlatformTarget::detect()
    }

    fn check_runtime(&self, id: &RuntimeId) -> InstallState {
        if self.runtime_dir(id).is_dir() {
            InstallState::Installed
        } else {
            InstallState::Missing
        }
    }

    fn install_runtime(&self, _id: &RuntimeId) -> CoreResult<()> {
        Err(CoreError::NotImplemented(
            "download/install pipeline lands in Phase 2 (see DEPENDENCY_STRATEGY.md §4-5)",
        ))
    }

    fn verify_runtime(&self, _id: &RuntimeId) -> CoreResult<()> {
        Err(CoreError::NotImplemented(
            "checksum verification lands in Phase 2 (see DEPENDENCY_STRATEGY.md §5)",
        ))
    }

    fn repair_runtime(&self, _id: &RuntimeId) -> CoreResult<()> {
        Err(CoreError::NotImplemented(
            "repair (reinstall) lands in Phase 2 (see DEPENDENCY_STRATEGY.md §6)",
        ))
    }
}
