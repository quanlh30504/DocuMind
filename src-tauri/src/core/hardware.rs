//! HardwareManager (ARCHITECTURE.md §2, spec §41): detects CPU/RAM/disk/arch so
//! DependencyManager and ModelManager (later phases) can pick compatible,
//! stability-appropriate runtime/model tiers.
//!
//! GPU/VRAM detection is deferred (see DEPENDENCY_STRATEGY.md §2 — NVML/Metal/DXGI
//! probing is platform-specific native work, not something `sysinfo` covers) and
//! currently always reports `None`, which the recommendation logic in
//! MODEL_STRATEGY.md treats as "assume CPU-only" rather than erroring.

use serde::{Deserialize, Serialize};
use sysinfo::{Disks, System};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PlatformTarget {
    WindowsX64,
    MacosArm64,
    MacosX64,
    LinuxX64,
    LinuxArm64,
    Unsupported,
}

impl PlatformTarget {
    pub fn detect() -> Self {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => Self::WindowsX64,
            ("macos", "aarch64") => Self::MacosArm64,
            ("macos", "x86_64") => Self::MacosX64,
            ("linux", "x86_64") => Self::LinuxX64,
            ("linux", "aarch64") => Self::LinuxArm64,
            _ => Self::Unsupported,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub vram_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub platform: PlatformTarget,
    pub cpu_cores: usize,
    pub ram_bytes: u64,
    pub disk_free_bytes: u64,
    /// `None` until platform-specific GPU probing lands (DEPENDENCY_STRATEGY.md §2).
    pub gpu: Option<GpuInfo>,
}

pub struct HardwareManager;

impl HardwareManager {
    pub fn profile() -> HardwareProfile {
        let mut sys = System::new_all();
        sys.refresh_all();

        let cpu_cores = sys.cpus().len();
        let ram_bytes = sys.total_memory();

        let disks = Disks::new_with_refreshed_list();
        let app_dir = crate::core::document::app_data_dir();
        let disk_free_bytes = disks
            .iter()
            .filter(|d| app_dir.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
            .map(|d| d.available_space())
            .unwrap_or_else(|| disks.iter().map(|d| d.available_space()).max().unwrap_or(0));

        HardwareProfile {
            platform: PlatformTarget::detect(),
            cpu_cores,
            ram_bytes,
            disk_free_bytes,
            gpu: None,
        }
    }
}
