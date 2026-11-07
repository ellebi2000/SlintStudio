//! Locating and installing the `slint-lsp` language server.
//!
//! Slint Studio manages its own copy of `slint-lsp`, pinned to the Slint
//! version used by the live preview (see [`slint_studio_core::install`] for
//! where it is installed and how downloads are verified). It is downloaded on
//! first use, or with `slint-studio --install-lsp`.

use std::path::PathBuf;

pub use slint_studio_core::install::{
    ArchiveFormat, BinarySource, InstallError, InstallProgress, LocatedBinary, ManagedTool,
    ReleaseAsset,
};

/// Version of `slint-lsp` installed by Slint Studio.
pub const SLINT_LSP_VERSION: &str = "1.18.1";

/// Environment variable that forces a specific `slint-lsp` executable.
pub const OVERRIDE_ENV: &str = "SLINT_STUDIO_LSP";

pub const BINARY_NAME: &str = if cfg!(windows) {
    "slint-lsp.exe"
} else {
    "slint-lsp"
};

/// The prebuilt `slint-lsp` archives of the pinned release.
pub const SLINT_LSP: ManagedTool = ManagedTool {
    name: "slint-lsp",
    binary_name: BINARY_NAME,
    version: SLINT_LSP_VERSION,
    override_env: OVERRIDE_ENV,
    assets: &[
        ReleaseAsset {
            os: "linux",
            arch: "x86_64",
            file_name: "slint-lsp-x86_64-unknown-linux-gnu.tar.gz",
            sha256: "62735d2b1908ae8a33b35dcdc71fd0c713f44ef8a4a9b57e8dfa07319903eefa",
            format: ArchiveFormat::TarGz,
        },
        ReleaseAsset {
            os: "linux",
            arch: "aarch64",
            file_name: "slint-lsp-aarch64-unknown-linux-gnu.tar.gz",
            sha256: "ac7ef43bf51b47f55b9220b471d5ae444baf1d8f6dae90c35edbfb57512985ee",
            format: ArchiveFormat::TarGz,
        },
        // Universal binary (x86_64 + arm64).
        ReleaseAsset {
            os: "macos",
            arch: "x86_64",
            file_name: "slint-lsp-macos.tar.gz",
            sha256: "5e9192d7798c0d825ce57893fce913426f2c5440814428fb6908a06616a1151f",
            format: ArchiveFormat::TarGz,
        },
        ReleaseAsset {
            os: "macos",
            arch: "aarch64",
            file_name: "slint-lsp-macos.tar.gz",
            sha256: "5e9192d7798c0d825ce57893fce913426f2c5440814428fb6908a06616a1151f",
            format: ArchiveFormat::TarGz,
        },
        ReleaseAsset {
            os: "windows",
            arch: "x86_64",
            file_name: "slint-lsp-windows-x86_64.zip",
            sha256: "4dc30c4b2024c5676092c647cb7471a1419a843fae46e632b50b68e2732fa912",
            format: ArchiveFormat::Zip,
        },
        ReleaseAsset {
            os: "windows",
            arch: "aarch64",
            file_name: "slint-lsp-windows-arm64.zip",
            sha256: "60bc5840c4c943c1dd0ddfc3767a05fa5c9284ad7b8199f56545cd17ebb2ccef",
            format: ArchiveFormat::Zip,
        },
    ],
};

/// Where the managed `slint-lsp` executable lives (whether or not it exists).
pub fn managed_binary() -> Option<PathBuf> {
    SLINT_LSP.managed_binary()
}

/// First `slint-lsp` found in the `PATH`.
pub fn find_in_path() -> Option<PathBuf> {
    SLINT_LSP.find_in_path()
}

/// The executable to use without downloading anything: the
/// [`OVERRIDE_ENV`] override if set, else the managed copy.
pub fn locate_installed() -> Option<LocatedBinary> {
    SLINT_LSP.locate_installed()
}

/// Downloads, verifies and installs the pinned `slint-lsp`. Returns the path
/// of the executable.
pub fn install(progress: &mut dyn FnMut(InstallProgress)) -> Result<PathBuf, InstallError> {
    SLINT_LSP.install(progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_platform_has_a_valid_asset() {
        assert_eq!(
            slint_studio_core::install::validate_tool(&SLINT_LSP),
            Ok(())
        );
        assert!(SLINT_LSP.asset_for("linux", "x86_64").is_some());
        assert!(SLINT_LSP.asset_for("windows", "x86_64").is_some());
        // The macOS archive is a universal binary.
        let arm = SLINT_LSP.asset_for("macos", "aarch64").unwrap();
        let intel = SLINT_LSP.asset_for("macos", "x86_64").unwrap();
        assert_eq!((arm.file_name, arm.sha256), (intel.file_name, intel.sha256));
        assert!(SLINT_LSP.asset_for("freebsd", "x86_64").is_none());
    }

    /// Downloads the real release: `cargo test -p slint-studio-lsp -- --ignored`.
    #[test]
    #[ignore = "requires network access"]
    fn downloads_and_verifies_the_real_release() {
        let dir = tempfile::tempdir().unwrap();
        let mut last = None;
        let binary = SLINT_LSP
            .install_into(dir.path(), &mut |p| last = Some(p))
            .unwrap();
        assert_eq!(last, Some(InstallProgress::Extracting));
        assert!(binary.is_file());
        assert!(dir.path().join("licenses/LICENSE.md").is_file());
        let output = std::process::Command::new(&binary)
            .arg("--version")
            .output()
            .unwrap();
        let version = String::from_utf8_lossy(&output.stdout);
        assert!(version.contains(SLINT_LSP_VERSION), "{version}");
    }
}
