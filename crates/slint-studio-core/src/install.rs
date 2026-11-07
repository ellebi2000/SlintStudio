//! Locating and installing the prebuilt Slint tools Slint Studio manages
//! (`slint-lsp`, `slint-viewer`).
//!
//! Each tool is pinned to the Slint version used by the live preview and
//! installed in the per-user data directory:
//!
//! | OS      | Location                                                         |
//! |---------|------------------------------------------------------------------|
//! | Linux   | `~/.local/share/slint-studio/<tool>/<version>/`                  |
//! | macOS   | `~/Library/Application Support/dev.slint-studio.slint-studio/…`  |
//! | Windows | `%APPDATA%\slint-studio\slint-studio\data\<tool>\<version>\`     |
//!
//! Tools are downloaded from the official Slint GitHub release on first use,
//! and every archive is checked against a SHA-256 checksum pinned in the
//! source code before anything is extracted.
//!
//! Only available with the `install` feature (it pulls in an HTTP client).

use std::env;
use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use directories::ProjectDirs;
use sha2::{Digest, Sha256};

use crate::app_dirs;

/// License files shipped in the release archives, extracted next to the binary.
const LICENSE_FILES: &[&str] = &["LICENSE.md", "THIRDPARTY.md"];

/// Refuse downloads larger than this.
const MAX_DOWNLOAD_SIZE: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    TarGz,
    Zip,
}

/// A prebuilt archive of a tool for one platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseAsset {
    /// `std::env::consts::OS` value, e.g. `"linux"`.
    pub os: &'static str,
    /// `std::env::consts::ARCH` value, e.g. `"x86_64"`.
    pub arch: &'static str,
    pub file_name: &'static str,
    pub sha256: &'static str,
    pub format: ArchiveFormat,
}

/// A command-line tool downloaded from a Slint release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedTool {
    /// Name of the tool and of its data directory, e.g. `"slint-lsp"`.
    pub name: &'static str,
    /// Executable file name (with `.exe` on Windows).
    pub binary_name: &'static str,
    /// Slint release the assets belong to.
    pub version: &'static str,
    /// Environment variable that forces a specific executable.
    pub override_env: &'static str,
    pub assets: &'static [ReleaseAsset],
}

impl ManagedTool {
    pub fn url(&self, asset: &ReleaseAsset) -> String {
        format!(
            "https://github.com/slint-ui/slint/releases/download/v{}/{}",
            self.version, asset.file_name
        )
    }

    pub fn asset_for(&self, os: &str, arch: &str) -> Option<ReleaseAsset> {
        self.assets
            .iter()
            .find(|asset| asset.os == os && asset.arch == arch)
            .copied()
    }

    /// The release asset for the platform we are running on.
    pub fn current_asset(&self) -> Option<ReleaseAsset> {
        self.asset_for(env::consts::OS, env::consts::ARCH)
    }

    /// Directory of the managed installation of the pinned version.
    pub fn managed_dir(&self) -> Option<PathBuf> {
        let dirs = ProjectDirs::from(
            app_dirs::QUALIFIER,
            app_dirs::ORGANIZATION,
            app_dirs::APPLICATION,
        )?;
        Some(dirs.data_dir().join(self.name).join(self.version))
    }

    /// Where the managed executable lives (whether or not it exists).
    pub fn managed_binary(&self) -> Option<PathBuf> {
        self.managed_dir().map(|dir| dir.join(self.binary_name))
    }

    /// First executable with this name found in the `PATH`.
    pub fn find_in_path(&self) -> Option<PathBuf> {
        find_in_path(self.binary_name)
    }

    /// The executable to use without downloading anything: the override if
    /// set (even if missing, so the error is visible), else the managed copy.
    pub fn locate_installed(&self) -> Option<LocatedBinary> {
        if let Some(path) = env::var_os(self.override_env).filter(|p| !p.is_empty()) {
            return Some(LocatedBinary {
                path: PathBuf::from(path),
                source: BinarySource::Override,
            });
        }
        self.managed_binary()
            .filter(|path| path.is_file())
            .map(|path| LocatedBinary {
                path,
                source: BinarySource::Managed,
            })
    }

    /// Downloads, verifies and installs the pinned release into
    /// [`managed_dir`](Self::managed_dir). Returns the path of the executable.
    pub fn install(
        &self,
        progress: &mut dyn FnMut(InstallProgress),
    ) -> Result<PathBuf, InstallError> {
        let dir = self.managed_dir().ok_or(InstallError::NoDataDirectory)?;
        self.install_into(&dir, progress)
    }

    /// Like [`install`](Self::install), into an arbitrary directory.
    pub fn install_into(
        &self,
        dir: &Path,
        progress: &mut dyn FnMut(InstallProgress),
    ) -> Result<PathBuf, InstallError> {
        let asset = self
            .current_asset()
            .ok_or(InstallError::UnsupportedPlatform {
                tool: self.name,
                os: env::consts::OS,
                arch: env::consts::ARCH,
            })?;
        let archive = download(&self.url(&asset), progress)?;
        progress(InstallProgress::Verifying);
        verify_sha256(&archive, asset.sha256)?;
        progress(InstallProgress::Extracting);
        install_from_archive(&archive, asset.format, self.binary_name, dir)
    }
}

/// First `binary_name` found in the `PATH`.
pub fn find_in_path(binary_name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(binary_name))
        .find(|candidate| candidate.is_file())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinarySource {
    /// From the tool's override environment variable.
    Override,
    /// Installed by Slint Studio.
    Managed,
    /// Found in the `PATH`.
    SystemPath,
}

impl BinarySource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Override => "variabile d'ambiente",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallProgress {
    Downloading { received: u64, total: Option<u64> },
    Verifying,
    Extracting,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("nessun {tool} precompilato per {os}/{arch}: installalo con `cargo install {tool}`")]
    UnsupportedPlatform {
        tool: &'static str,
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

/// Extracts the executable `binary_name` (and license files) from a release
/// archive into `dir`. Archive paths are never used as output paths: only
/// entries whose file name matches are written, to fixed names inside `dir`.
pub fn install_from_archive(
    archive: &[u8],
    format: ArchiveFormat,
    binary_name: &str,
    dir: &Path,
) -> Result<PathBuf, InstallError> {
    fs::create_dir_all(dir.join("licenses"))?;
    // A unique temporary name: another instance may be installing too.
    let partial = dir.join(format!("{binary_name}.partial-{}", std::process::id()));
    let mut found_binary = false;

    let mut handle_entry = |name: &str, reader: &mut dyn Read| -> Result<(), InstallError> {
        let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let target = if file_name == binary_name {
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
            "{binary_name} non trovato nell'archivio"
        )));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o755))?;
    }
    let binary = dir.join(binary_name);
    fs::rename(&partial, &binary)?;
    Ok(binary)
}

fn archive_error(err: impl std::fmt::Display) -> InstallError {
    InstallError::Archive(err.to_string())
}

/// Checks the invariants every [`ManagedTool`] definition must satisfy;
/// called from the unit tests of the crates defining tools.
pub fn validate_tool(tool: &ManagedTool) -> Result<(), String> {
    if tool.assets.is_empty() {
        return Err(format!("{}: nessun asset", tool.name));
    }
    let expected_binary = if cfg!(windows) {
        format!("{}.exe", tool.name)
    } else {
        tool.name.to_owned()
    };
    if tool.binary_name != expected_binary {
        return Err(format!(
            "{}: nome eseguibile {}",
            tool.name, tool.binary_name
        ));
    }
    for asset in tool.assets {
        let id = format!("{} {}/{}", tool.name, asset.os, asset.arch);
        if asset.sha256.len() != 64 || !asset.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("{id}: checksum non valido"));
        }
        if !asset.file_name.starts_with(tool.name) {
            return Err(format!("{id}: archivio {}", asset.file_name));
        }
        let zip = asset.file_name.ends_with(".zip");
        if zip != (asset.format == ArchiveFormat::Zip) {
            return Err(format!("{id}: formato non coerente"));
        }
        let duplicates = tool
            .assets
            .iter()
            .filter(|a| a.os == asset.os && a.arch == asset.arch)
            .count();
        if duplicates != 1 {
            return Err(format!("{id}: piattaforma duplicata"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BINARY: &str = "tool";

    const TOOL: ManagedTool = ManagedTool {
        name: "slint-demo",
        binary_name: if cfg!(windows) {
            "slint-demo.exe"
        } else {
            "slint-demo"
        },
        version: "1.2.3",
        override_env: "SLINT_STUDIO_TEST_TOOL_THAT_IS_NEVER_SET",
        assets: &[
            ReleaseAsset {
                os: "linux",
                arch: "x86_64",
                file_name: "slint-demo-linux.tar.gz",
                sha256: "62735d2b1908ae8a33b35dcdc71fd0c713f44ef8a4a9b57e8dfa07319903eefa",
                format: ArchiveFormat::TarGz,
            },
            ReleaseAsset {
                os: "windows",
                arch: "x86_64",
                file_name: "slint-demo-windows.zip",
                sha256: "4dc30c4b2024c5676092c647cb7471a1419a843fae46e632b50b68e2732fa912",
                format: ArchiveFormat::Zip,
            },
        ],
    };

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
    fn tool_metadata() {
        assert_eq!(validate_tool(&TOOL), Ok(()));
        let asset = TOOL.asset_for("linux", "x86_64").unwrap();
        assert_eq!(
            TOOL.url(&asset),
            "https://github.com/slint-ui/slint/releases/download/v1.2.3/slint-demo-linux.tar.gz"
        );
        assert!(TOOL.asset_for("freebsd", "x86_64").is_none());
        let managed = TOOL.managed_binary().unwrap();
        assert!(managed.ends_with(Path::new("slint-demo/1.2.3").join(TOOL.binary_name)));
        assert_eq!(TOOL.locate_installed(), None, "nothing installed");
    }

    #[test]
    fn invalid_tools_are_reported() {
        let mut broken = TOOL;
        broken.assets = &[ReleaseAsset {
            os: "linux",
            arch: "x86_64",
            file_name: "slint-demo.zip",
            sha256: "62735d2b1908ae8a33b35dcdc71fd0c713f44ef8a4a9b57e8dfa07319903eefa",
            format: ArchiveFormat::TarGz,
        }];
        assert!(validate_tool(&broken).is_err());
        broken.assets = &[];
        assert!(validate_tool(&broken).is_err());
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
            ("tool/licenses/LICENSE.md", b"license"),
            ("tool/licenses/THIRDPARTY.md", b"third party"),
            ("tool/tool", b"binary"),
            ("tool/README.txt", b"ignored"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let binary =
            install_from_archive(&archive, ArchiveFormat::TarGz, BINARY, dir.path()).unwrap();
        assert_eq!(binary, dir.path().join(BINARY));
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
        let archive = zip(&[
            ("licenses\\LICENSE.md", b"license"),
            ("tool.exe", b"exe"),
            ("tool", b"other"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let binary =
            install_from_archive(&archive, ArchiveFormat::Zip, "tool.exe", dir.path()).unwrap();
        assert_eq!(fs::read(binary).unwrap(), b"exe");
        assert_eq!(
            fs::read(dir.path().join("licenses/LICENSE.md")).unwrap(),
            b"license"
        );
        assert!(!dir.path().join("tool").exists());
    }

    #[test]
    fn path_traversal_entries_cannot_escape_the_install_dir() {
        // `tar::Builder` refuses `..`, so write the malicious name by hand.
        let name = format!("../../evil/{BINARY}");
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
        let binary = install_from_archive(&archive, ArchiveFormat::TarGz, BINARY, &dir).unwrap();
        assert_eq!(binary, dir.join(BINARY));
        assert!(!outer.path().join("evil").exists());
    }

    #[test]
    fn archive_without_binary_is_rejected() {
        let archive = tar_gz(&[("tool/licenses/LICENSE.md", b"license")]);
        let dir = tempfile::tempdir().unwrap();
        let err =
            install_from_archive(&archive, ArchiveFormat::TarGz, BINARY, dir.path()).unwrap_err();
        assert!(matches!(err, InstallError::Archive(_)));
        assert!(!dir.path().join(BINARY).exists());
    }

    #[test]
    fn garbage_is_not_an_archive() {
        let dir = tempfile::tempdir().unwrap();
        for format in [ArchiveFormat::Zip, ArchiveFormat::TarGz] {
            assert!(install_from_archive(b"not an archive", format, BINARY, dir.path()).is_err());
        }
    }
}
