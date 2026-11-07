//! Locating and installing the `slint-lsp` language server.
//!
//! Slint Studio manages its own copy of `slint-lsp`, pinned to the Slint
//! version used by the live preview, in the per-user data directory:
//!
//! | OS      | Location                                                        |
//! |---------|-----------------------------------------------------------------|
//! | Linux   | `~/.local/share/slint-studio/slint-lsp/<version>/`              |
//! | macOS   | `~/Library/Application Support/dev.slint-studio.slint-studio/…` |
//! | Windows | `%APPDATA%\slint-studio\slint-studio\data\slint-lsp\<version>\` |
//!
//! It is downloaded from the official Slint GitHub release on first use (or
//! with `slint-studio --install-lsp`), and the archive is checked against a
//! SHA-256 checksum pinned in this file before anything is extracted.

use std::env;
use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use directories::ProjectDirs;
use sha2::{Digest, Sha256};
use slint_studio_core::app_dirs;

/// Version of `slint-lsp` installed by Slint Studio.
pub const SLINT_LSP_VERSION: &str = "1.18.1";

/// Environment variable that forces a specific `slint-lsp` executable.
pub const OVERRIDE_ENV: &str = "SLINT_STUDIO_LSP";

pub const BINARY_NAME: &str = if cfg!(windows) {
    "slint-lsp.exe"
} else {
    "slint-lsp"
};

/// License files shipped in the release archives, extracted next to the binary.
const LICENSE_FILES: &[&str] = &["LICENSE.md", "THIRDPARTY.md"];

/// Refuse downloads larger than this.
const MAX_DOWNLOAD_SIZE: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    TarGz,
    Zip,
}

/// A prebuilt `slint-lsp` archive of the pinned release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub file_name: &'static str,
    pub sha256: &'static str,
    pub format: ArchiveFormat,
}

impl ReleaseAsset {
    pub fn url(&self) -> String {
        format!(
            "https://github.com/slint-ui/slint/releases/download/v{SLINT_LSP_VERSION}/{}",
            self.file_name
        )
    }
}

/// `(target_os, target_arch, asset)` for every supported platform.
const ASSETS: &[(&str, &str, ReleaseAsset)] = &[
    (
        "linux",
        "x86_64",
        ReleaseAsset {
            file_name: "slint-lsp-x86_64-unknown-linux-gnu.tar.gz",
            sha256: "62735d2b1908ae8a33b35dcdc71fd0c713f44ef8a4a9b57e8dfa07319903eefa",
            format: ArchiveFormat::TarGz,
        },
    ),
    (
        "linux",
        "aarch64",
        ReleaseAsset {
            file_name: "slint-lsp-aarch64-unknown-linux-gnu.tar.gz",
            sha256: "ac7ef43bf51b47f55b9220b471d5ae444baf1d8f6dae90c35edbfb57512985ee",
            format: ArchiveFormat::TarGz,
        },
    ),
    // Universal binary (x86_64 + arm64).
    (
        "macos",
        "x86_64",
        ReleaseAsset {
            file_name: "slint-lsp-macos.tar.gz",
            sha256: "5e9192d7798c0d825ce57893fce913426f2c5440814428fb6908a06616a1151f",
            format: ArchiveFormat::TarGz,
        },
    ),
    (
        "macos",
        "aarch64",
        ReleaseAsset {
            file_name: "slint-lsp-macos.tar.gz",
            sha256: "5e9192d7798c0d825ce57893fce913426f2c5440814428fb6908a06616a1151f",
            format: ArchiveFormat::TarGz,
        },
    ),
    (
        "windows",
        "x86_64",
        ReleaseAsset {
            file_name: "slint-lsp-windows-x86_64.zip",
            sha256: "4dc30c4b2024c5676092c647cb7471a1419a843fae46e632b50b68e2732fa912",
            format: ArchiveFormat::Zip,
        },
    ),
    (
        "windows",
        "aarch64",
        ReleaseAsset {
            file_name: "slint-lsp-windows-arm64.zip",
            sha256: "60bc5840c4c943c1dd0ddfc3767a05fa5c9284ad7b8199f56545cd17ebb2ccef",
            format: ArchiveFormat::Zip,
        },
    ),
];

pub fn asset_for(os: &str, arch: &str) -> Option<ReleaseAsset> {
    ASSETS
        .iter()
        .find(|(o, a, _)| *o == os && *a == arch)
        .map(|(_, _, asset)| *asset)
}

/// The release asset for the platform we are running on.
pub fn current_asset() -> Option<ReleaseAsset> {
    asset_for(env::consts::OS, env::consts::ARCH)
}

/// Directory of the managed installation of the pinned version.
pub fn managed_dir() -> Option<PathBuf> {
    let dirs = ProjectDirs::from(
        app_dirs::QUALIFIER,
        app_dirs::ORGANIZATION,
        app_dirs::APPLICATION,
    )?;
    Some(dirs.data_dir().join("slint-lsp").join(SLINT_LSP_VERSION))
}

/// Where the managed `slint-lsp` executable lives (whether or not it exists).
pub fn managed_binary() -> Option<PathBuf> {
    managed_dir().map(|dir| dir.join(BINARY_NAME))
}

/// First `slint-lsp` found in the `PATH`.
pub fn find_in_path() -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(BINARY_NAME))
        .find(|candidate| candidate.is_file())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinarySource {
    /// From the [`OVERRIDE_ENV`] environment variable.
    Override,
    /// Installed by Slint Studio.
    Managed,
    /// Found in the `PATH`.
    SystemPath,
}

impl BinarySource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Override => OVERRIDE_ENV,
            Self::Managed => "gestito da Slint Studio",
            Self::SystemPath => "PATH di sistema",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedBinary {
    pub path: PathBuf,
    pub source: BinarySource,
}

/// The executable to use without downloading anything: the override if set
/// (even if missing, so the error is visible), else the managed copy.
pub fn locate_installed() -> Option<LocatedBinary> {
    if let Some(path) = env::var_os(OVERRIDE_ENV).filter(|p| !p.is_empty()) {
        return Some(LocatedBinary {
            path: PathBuf::from(path),
            source: BinarySource::Override,
        });
    }
    managed_binary()
        .filter(|path| path.is_file())
        .map(|path| LocatedBinary {
            path,
            source: BinarySource::Managed,
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallProgress {
    Downloading { received: u64, total: Option<u64> },
    Verifying,
    Extracting,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error(
        "nessun slint-lsp precompilato per {os}/{arch}: installalo con `cargo install slint-lsp`"
    )]
    UnsupportedPlatform {
        os: &'static str,
        arch: &'static str,
    },
    #[error("cartella dati utente non disponibile")]
    NoDataDirectory,
    #[error("download non riuscito: {0}")]
    Download(String),
    #[error(
        "checksum non valido (atteso {expected}, ottenuto {actual}): download corrotto o manomesso"
    )]
    Checksum { expected: String, actual: String },
    #[error("archivio non valido: {0}")]
    Archive(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Downloads, verifies and installs the pinned `slint-lsp` into
/// [`managed_dir`]. Returns the path of the executable.
pub fn install(progress: &mut dyn FnMut(InstallProgress)) -> Result<PathBuf, InstallError> {
    let dir = managed_dir().ok_or(InstallError::NoDataDirectory)?;
    install_into(&dir, progress)
}

/// Like [`install`], into an arbitrary directory.
pub fn install_into(
    dir: &Path,
    progress: &mut dyn FnMut(InstallProgress),
) -> Result<PathBuf, InstallError> {
    let asset = current_asset().ok_or(InstallError::UnsupportedPlatform {
        os: env::consts::OS,
        arch: env::consts::ARCH,
    })?;
    let archive = download(&asset.url(), progress)?;
    progress(InstallProgress::Verifying);
    verify_sha256(&archive, asset.sha256)?;
    progress(InstallProgress::Extracting);
    install_from_archive(&archive, asset.format, dir)
}

fn download(url: &str, progress: &mut dyn FnMut(InstallProgress)) -> Result<Vec<u8>, InstallError> {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_per_read(Some(Duration::from_secs(60)))
        .user_agent(concat!("slint-studio/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();

    let mut response = agent
        .get(url)
        .call()
        .map_err(|err| InstallError::Download(err.to_string()))?;
    let total = response.body().content_length();
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_SIZE)
        .reader();

    let capacity = total.map_or(0, |t| {
        usize::try_from(t.min(MAX_DOWNLOAD_SIZE)).unwrap_or(0)
    });
    let mut data = Vec::with_capacity(capacity);
    let mut chunk = vec![0; 64 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|err| InstallError::Download(err.to_string()))?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..read]);
        progress(InstallProgress::Downloading {
            received: data.len() as u64,
            total,
        });
    }
    Ok(data)
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn verify_sha256(data: &[u8], expected: &str) -> Result<(), InstallError> {
    let actual = sha256_hex(data);
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(InstallError::Checksum {
            expected: expected.to_owned(),
            actual,
        })
    }
}

/// Extracts the executable (and license files) from a release archive into
/// `dir`. Archive paths are never used as output paths: only entries whose
/// file name matches are written, to fixed names inside `dir`.
pub fn install_from_archive(
    archive: &[u8],
    format: ArchiveFormat,
    dir: &Path,
) -> Result<PathBuf, InstallError> {
    fs::create_dir_all(dir.join("licenses"))?;
    // A unique temporary name: another instance may be installing too.
    let partial = dir.join(format!("{BINARY_NAME}.partial-{}", std::process::id()));
    let mut found_binary = false;

    let mut handle_entry = |name: &str, reader: &mut dyn Read| -> Result<(), InstallError> {
        let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let target = if file_name == BINARY_NAME {
            found_binary = true;
            partial.clone()
        } else if LICENSE_FILES.contains(&file_name) {
            dir.join("licenses").join(file_name)
        } else {
            return Ok(());
        };
        let mut file = fs::File::create(&target)?;
        io::copy(reader, &mut file)?;
        file.flush()?;
        Ok(())
    };

    match format {
        ArchiveFormat::TarGz => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(archive)));
            for entry in tar.entries().map_err(archive_error)? {
                let mut entry = entry.map_err(archive_error)?;
                if !entry.header().entry_type().is_file() {
                    continue;
                }
                let name = entry
                    .path()
                    .map_err(archive_error)?
                    .to_string_lossy()
                    .into_owned();
                handle_entry(&name, &mut entry)?;
            }
        }
        ArchiveFormat::Zip => {
            let mut zip = zip::ZipArchive::new(Cursor::new(archive)).map_err(archive_error)?;
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index).map_err(archive_error)?;
                if entry.is_dir() {
                    continue;
                }
                let name = entry.name().map_err(archive_error)?.into_owned();
                handle_entry(&name, &mut entry)?;
            }
        }
    }

    if !found_binary {
        let _ = fs::remove_file(&partial);
        return Err(InstallError::Archive(format!(
            "{BINARY_NAME} non trovato nell'archivio"
        )));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o755))?;
    }
    let binary = dir.join(BINARY_NAME);
    fs::rename(&partial, &binary)?;
    Ok(binary)
}

fn archive_error(err: impl std::fmt::Display) -> InstallError {
    InstallError::Archive(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        for (path, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, data) in entries {
            writer
                .start_file(*path, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn every_supported_platform_has_a_valid_checksum() {
        for (os, arch, asset) in ASSETS {
            assert_eq!(asset.sha256.len(), 64, "{os}/{arch}");
            assert!(asset.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(
                asset
                    .url()
                    .starts_with("https://github.com/slint-ui/slint/releases/download/v")
            );
            let zip = asset.file_name.ends_with(".zip");
            assert_eq!(zip, asset.format == ArchiveFormat::Zip);
        }
        assert!(asset_for("linux", "x86_64").is_some());
        assert!(asset_for("windows", "x86_64").is_some());
        assert_eq!(asset_for("macos", "aarch64"), asset_for("macos", "x86_64"));
        assert!(asset_for("freebsd", "x86_64").is_none());
    }

    #[test]
    fn checksum_verification() {
        let data = b"slint";
        let hash = sha256_hex(data);
        assert_eq!(hash.len(), 64);
        assert!(verify_sha256(data, &hash).is_ok());
        assert!(verify_sha256(data, &hash.to_uppercase()).is_ok());
        assert!(matches!(
            verify_sha256(b"tampered", &hash),
            Err(InstallError::Checksum { .. })
        ));
    }

    #[test]
    fn extracts_binary_and_licenses_from_tar_gz() {
        let archive = tar_gz(&[
            ("slint-lsp/licenses/LICENSE.md", b"license"),
            ("slint-lsp/licenses/THIRDPARTY.md", b"third party"),
            (&format!("slint-lsp/{BINARY_NAME}"), b"binary"),
            ("slint-lsp/README.txt", b"ignored"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let binary = install_from_archive(&archive, ArchiveFormat::TarGz, dir.path()).unwrap();
        assert_eq!(binary, dir.path().join(BINARY_NAME));
        assert_eq!(fs::read(&binary).unwrap(), b"binary");
        assert_eq!(
            fs::read(dir.path().join("licenses/LICENSE.md")).unwrap(),
            b"license"
        );
        assert!(!dir.path().join("README.txt").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&binary).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "binary must be executable");
        }
    }

    #[test]
    fn extracts_from_flat_zip_with_backslashes() {
        // Layout of the Windows release archives.
        let archive = zip(&[("licenses\\LICENSE.md", b"license"), (BINARY_NAME, b"exe")]);
        let dir = tempfile::tempdir().unwrap();
        let binary = install_from_archive(&archive, ArchiveFormat::Zip, dir.path()).unwrap();
        assert_eq!(fs::read(binary).unwrap(), b"exe");
        assert_eq!(
            fs::read(dir.path().join("licenses/LICENSE.md")).unwrap(),
            b"license"
        );
    }

    #[test]
    fn path_traversal_entries_cannot_escape_the_install_dir() {
        // `tar::Builder` refuses `..`, so write the malicious name by hand.
        let name = format!("../../evil/{BINARY_NAME}");
        let mut header = tar::Header::new_old();
        header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        header.set_size(7);
        header.set_mode(0o644);
        header.set_cksum();
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        builder.append(&header, &b"payload"[..]).unwrap();
        let archive = builder.into_inner().unwrap().finish().unwrap();

        let outer = tempfile::tempdir().unwrap();
        let dir = outer.path().join("a/b");
        let binary = install_from_archive(&archive, ArchiveFormat::TarGz, &dir).unwrap();
        assert_eq!(binary, dir.join(BINARY_NAME));
        assert!(!outer.path().join("evil").exists());
    }

    #[test]
    fn archive_without_binary_is_rejected() {
        let archive = tar_gz(&[("slint-lsp/licenses/LICENSE.md", b"license")]);
        let dir = tempfile::tempdir().unwrap();
        let err = install_from_archive(&archive, ArchiveFormat::TarGz, dir.path()).unwrap_err();
        assert!(matches!(err, InstallError::Archive(_)));
        assert!(!dir.path().join(BINARY_NAME).exists());
    }

    /// Downloads the real release: `cargo test -p slint-studio-lsp -- --ignored`.
    #[test]
    #[ignore = "requires network access"]
    fn downloads_and_verifies_the_real_release() {
        let dir = tempfile::tempdir().unwrap();
        let mut last = None;
        let binary = install_into(dir.path(), &mut |p| last = Some(p)).unwrap();
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

    #[test]
    fn garbage_is_not_an_archive() {
        let dir = tempfile::tempdir().unwrap();
        assert!(install_from_archive(b"not an archive", ArchiveFormat::Zip, dir.path()).is_err());
        assert!(install_from_archive(b"not an archive", ArchiveFormat::TarGz, dir.path()).is_err());
    }
}
