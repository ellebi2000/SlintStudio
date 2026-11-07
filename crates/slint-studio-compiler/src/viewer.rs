//! The `slint-viewer` used by "Esegui" for projects made only of `.slint`
//! files.
//!
//! Like `slint-lsp`, it is pinned to the Slint version of the live preview,
//! downloaded from the official release on first use and verified with the
//! SHA-256 checksums below (see [`slint_studio_core::install`]).

use std::path::Path;

use slint_studio_core::install::{ArchiveFormat, ManagedTool, ReleaseAsset};

use crate::process::ProcessCommand;

/// Version of `slint-viewer` installed by Slint Studio.
pub const SLINT_VIEWER_VERSION: &str = "1.18.1";

/// Environment variable that forces a specific `slint-viewer` executable.
pub const OVERRIDE_ENV: &str = "SLINT_STUDIO_VIEWER";

pub const BINARY_NAME: &str = if cfg!(windows) {
    "slint-viewer.exe"
} else {
    "slint-viewer"
};

pub const SLINT_VIEWER: ManagedTool = ManagedTool {
    name: "slint-viewer",
    binary_name: BINARY_NAME,
    version: SLINT_VIEWER_VERSION,
    override_env: OVERRIDE_ENV,
    assets: &[
        ReleaseAsset {
            os: "linux",
            arch: "x86_64",
            file_name: "slint-viewer-x86_64-unknown-linux-gnu.tar.gz",
            sha256: "86b260e661cdc24dcadaf63266b034f3281cbec542afc26545a72a986fe42a13",
            format: ArchiveFormat::TarGz,
        },
        ReleaseAsset {
            os: "linux",
            arch: "aarch64",
            file_name: "slint-viewer-aarch64-unknown-linux-gnu.tar.gz",
            sha256: "7aad0325b6af939c4adefd49bd17611511dca056c89748e7ded9d45c062953da",
            format: ArchiveFormat::TarGz,
        },
        // Universal binary (x86_64 + arm64).
        ReleaseAsset {
            os: "macos",
            arch: "x86_64",
            file_name: "slint-viewer-macos.tar.gz",
            sha256: "e6ced86250e052122b65b593c77c2042a54798428a0f9ae5840f8769f1488571",
            format: ArchiveFormat::TarGz,
        },
        ReleaseAsset {
            os: "macos",
            arch: "aarch64",
            file_name: "slint-viewer-macos.tar.gz",
            sha256: "e6ced86250e052122b65b593c77c2042a54798428a0f9ae5840f8769f1488571",
            format: ArchiveFormat::TarGz,
        },
        ReleaseAsset {
            os: "windows",
            arch: "x86_64",
            file_name: "slint-viewer-windows-x86_64.zip",
            sha256: "1bdad00c098ce5ca5f9c0e6ccb666170b43c8fdcec20a4e6e7c1539f5d38818e",
            format: ArchiveFormat::Zip,
        },
        ReleaseAsset {
            os: "windows",
            arch: "aarch64",
            file_name: "slint-viewer-windows-arm64.zip",
            sha256: "522e40da5fc30071e838a1d0459f8aeb4d6d2d49c7cd3c202972ca229daeb67b",
            format: ArchiveFormat::Zip,
        },
    ],
};

/// The command that opens `file` in a window, reloading it when it changes
/// on disk (after "Salva").
pub fn command(viewer: &Path, file: &Path, project_root: &Path, style: &str) -> ProcessCommand {
    ProcessCommand::new(viewer, project_root)
        .arg("--auto-reload")
        .arg("--style")
        .arg(style)
        .arg(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_platform_has_a_valid_asset() {
        assert_eq!(
            slint_studio_core::install::validate_tool(&SLINT_VIEWER),
            Ok(())
        );
        assert!(SLINT_VIEWER.asset_for("linux", "x86_64").is_some());
        assert!(SLINT_VIEWER.asset_for("windows", "aarch64").is_some());
        let arm = SLINT_VIEWER.asset_for("macos", "aarch64").unwrap();
        let intel = SLINT_VIEWER.asset_for("macos", "x86_64").unwrap();
        assert_eq!((arm.file_name, arm.sha256), (intel.file_name, intel.sha256));
        assert!(
            SLINT_VIEWER
                .managed_binary()
                .unwrap()
                .ends_with(Path::new("slint-viewer/1.18.1").join(BINARY_NAME))
        );
    }

    #[test]
    fn viewer_command_line() {
        let command = command(
            Path::new("/opt/slint-viewer"),
            Path::new("/p/ui/main.slint"),
            Path::new("/p"),
            "fluent",
        );
        assert_eq!(
            command.display(),
            "slint-viewer --auto-reload --style fluent /p/ui/main.slint"
        );
        assert_eq!(command.cwd, Path::new("/p"));
    }

    /// Downloads the real release: `cargo test -p slint-studio-compiler -- --ignored`.
    #[test]
    #[ignore = "requires network access"]
    fn downloads_and_verifies_the_real_release() {
        let dir = tempfile::tempdir().unwrap();
        let binary = SLINT_VIEWER.install_into(dir.path(), &mut |_| {}).unwrap();
        let output = std::process::Command::new(&binary)
            .arg("--version")
            .output()
            .unwrap();
        let version = String::from_utf8_lossy(&output.stdout);
        assert!(version.contains(SLINT_VIEWER_VERSION), "{version}");
    }
}
