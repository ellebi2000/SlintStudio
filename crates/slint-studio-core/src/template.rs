//! Templates for "Nuovo progetto" (new project).
//!
//! Two kinds of project, the ones "Compila" and "Esegui" support:
//! - [`ProjectTemplate::RustSlint`]: a Cargo application laid out like the
//!   official Slint Rust template (`build.rs` compiling `ui/app-window.slint`
//!   with `slint-build`);
//! - [`ProjectTemplate::SlintOnly`]: just `.slint` files, run with
//!   `slint-viewer`.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Slint version used by new Rust projects: the one of the live preview and
/// of the managed tools.
pub const SLINT_VERSION: &str = "1.18.1";

/// The file opened after creating a project (also the entry point).
pub const MAIN_FILE: &str = "ui/app-window.slint";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectTemplate {
    /// Rust application with a Slint UI, built and run with cargo.
    #[default]
    RustSlint,
    /// Only `.slint` files, opened with `slint-viewer`.
    SlintOnly,
}

impl ProjectTemplate {
    pub const ALL: [Self; 2] = [Self::RustSlint, Self::SlintOnly];

    pub fn label(self) -> &'static str {
        match self {
            Self::RustSlint => "Applicazione Rust + Slint",
            Self::SlintOnly => "Solo Slint",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::RustSlint => {
                "Progetto Cargo con l'interfaccia in ui/app-window.slint, compilata da \
                 build.rs con slint-build. Compila ed Esegui usano cargo."
            }
            Self::SlintOnly => {
                "Solo file .slint, senza codice Rust: ideale per disegnare interfacce. \
                 Esegui li apre con slint-viewer."
            }
        }
    }
}

/// Why a project name or location cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Empty,
    /// Contains `/`, `\` or another character not allowed in folder names.
    InvalidCharacter(char),
    /// `.` or `..`, or starts with a dot (hidden folder).
    Reserved,
    /// The Cargo package name derived from the project name would be empty
    /// (no letters or digits).
    NoPackageName,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "inserisci un nome"),
            Self::InvalidCharacter(c) => write!(f, "il nome non può contenere «{c}»"),
            Self::Reserved => write!(f, "il nome non può iniziare con un punto"),
            Self::NoPackageName => write!(f, "il nome deve contenere almeno una lettera o cifra"),
        }
    }
}

/// Characters not allowed in folder names on at least one supported OS.
const FORBIDDEN: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Checks a project (folder) name.
pub fn validate_name(name: &str, template: ProjectTemplate) -> Result<(), NameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if let Some(c) = name
        .chars()
        .find(|c| FORBIDDEN.contains(c) || c.is_control())
    {
        return Err(NameError::InvalidCharacter(c));
    }
    if name.starts_with('.') {
        return Err(NameError::Reserved);
    }
    if template == ProjectTemplate::RustSlint && package_name(name).is_none() {
        return Err(NameError::NoPackageName);
    }
    Ok(())
}

/// Rust keywords and names cargo refuses as package names.
const RESERVED_PACKAGES: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "abstract",
    "become",
    "box",
    "do",
    "final",
    "macro",
    "override",
    "priv",
    "try",
    "typeof",
    "unsized",
    "virtual",
    "yield",
    "test",
    "core",
    "std",
    "alloc",
    "proc_macro",
    "proc-macro",
];

/// The Cargo package name for a project name: lowercase ASCII letters,
/// digits, `-` and `_`; spaces become `-`; accents are dropped. `None` if
/// nothing usable is left.
pub fn package_name(project_name: &str) -> Option<String> {
    let mut name = String::new();
    for c in project_name.trim().chars() {
        let c = fold_accent(c).to_ascii_lowercase();
        match c {
            'a'..='z' | '0'..='9' | '_' => name.push(c),
            '-' | ' ' | '.' if !name.is_empty() && !name.ends_with('-') => name.push('-'),
            _ => {}
        }
    }
    let name = name.trim_end_matches('-').to_owned();
    if name.is_empty() {
        return None;
    }
    // Package names cannot start with a digit, nor be keywords.
    if name.starts_with(|c: char| c.is_ascii_digit()) || RESERVED_PACKAGES.contains(&name.as_str())
    {
        return Some(format!("app-{name}"));
    }
    Some(name)
}

/// Italian (and common European) accented letters to their base letter.
fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' | 'À' | 'Á' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'a',
        'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'i',
        'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'Ò' | 'Ó' | 'Ô' | 'Ö' | 'Õ' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'u',
        'ç' | 'Ç' => 'c',
        'ñ' | 'Ñ' => 'n',
        other => other,
    }
}

/// The window title: the project name as typed.
fn window_title(project_name: &str) -> String {
    project_name
        .trim()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// The files of a new project, as `(relative path, content)`.
pub fn files(template: ProjectTemplate, project_name: &str) -> Vec<(PathBuf, String)> {
    let title = window_title(project_name);
    match template {
        ProjectTemplate::SlintOnly => vec![(PathBuf::from(MAIN_FILE), slint_only_ui(&title))],
        ProjectTemplate::RustSlint => {
            let package = package_name(project_name).unwrap_or_else(|| "app".to_owned());
            vec![
                (PathBuf::from("Cargo.toml"), cargo_toml(&package)),
                (PathBuf::from("build.rs"), BUILD_RS.to_owned()),
                (PathBuf::from("src/main.rs"), MAIN_RS.to_owned()),
                (PathBuf::from(MAIN_FILE), rust_ui(&title)),
                (PathBuf::from(".gitignore"), "/target\n".to_owned()),
            ]
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    #[error("{0}")]
    InvalidName(NameError),
    #[error("la cartella {} non esiste", .0.display())]
    NoParent(PathBuf),
    #[error("la cartella {} esiste già e non è vuota", .0.display())]
    NotEmpty(PathBuf),
    #[error("impossibile creare il progetto: {0}")]
    Io(#[from] io::Error),
}

/// Where the project would be created, or why it cannot be.
pub fn target_dir(
    parent: &Path,
    project_name: &str,
    template: ProjectTemplate,
) -> Result<PathBuf, CreateError> {
    validate_name(project_name, template).map_err(CreateError::InvalidName)?;
    if !parent.is_dir() {
        return Err(CreateError::NoParent(parent.to_path_buf()));
    }
    let dir = parent.join(project_name.trim());
    if dir.exists() && !is_empty_dir(&dir) {
        return Err(CreateError::NotEmpty(dir));
    }
    Ok(dir)
}

fn is_empty_dir(path: &Path) -> bool {
    fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
}

/// Creates the project folder `parent/project_name` with the template's
/// files. Returns the folder.
pub fn create(
    parent: &Path,
    project_name: &str,
    template: ProjectTemplate,
) -> Result<PathBuf, CreateError> {
    let dir = target_dir(parent, project_name, template)?;
    fs::create_dir_all(&dir)?;
    for (relative, content) in files(template, project_name) {
        let path = dir.join(relative);
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        fs::write(&path, content)?;
    }
    Ok(dir)
}

fn cargo_toml(package: &str) -> String {
    format!(
        r#"[package]
name = "{package}"
version = "0.1.0"
edition = "2024"

[dependencies]
slint = "{SLINT_VERSION}"

[build-dependencies]
slint-build = "{SLINT_VERSION}"
"#
    )
}

const BUILD_RS: &str = r#"fn main() {
    slint_build::compile("ui/app-window.slint").expect("Slint build failed");
}
"#;

const MAIN_RS: &str = r#"// Prevent a console window in addition to the Slint window in Windows release
// builds (e.g. when started from the file manager). Ignored on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::error::Error;

slint::include_modules!();

fn main() -> Result<(), Box<dyn Error>> {
    let ui = AppWindow::new()?;

    ui.on_request_increase_value({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_counter(ui.get_counter() + 1);
            }
        }
    });

    ui.run()?;
    Ok(())
}
"#;

fn rust_ui(title: &str) -> String {
    format!(
        r#"import {{ Button, VerticalBox }} from "std-widgets.slint";

export component AppWindow inherits Window {{
    title: "{title}";
    in-out property <int> counter: 42;
    callback request-increase-value();

    VerticalBox {{
        Text {{
            text: "Contatore: \{{root.counter}}";
            horizontal-alignment: center;
        }}
        Button {{
            text: "Aumenta";
            clicked => {{
                root.request-increase-value();
            }}
        }}
    }}
}}
"#
    )
}

fn slint_only_ui(title: &str) -> String {
    format!(
        r#"import {{ Button, VerticalBox }} from "std-widgets.slint";

export component AppWindow inherits Window {{
    title: "{title}";
    in-out property <int> counter: 42;

    VerticalBox {{
        Text {{
            text: "Contatore: \{{root.counter}}";
            horizontal-alignment: center;
        }}
        Button {{
            text: "Aumenta";
            clicked => {{
                root.counter += 1;
            }}
        }}
    }}
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_follow_cargo_rules() {
        assert_eq!(package_name("Contatore").as_deref(), Some("contatore"));
        assert_eq!(package_name("La mia App").as_deref(), Some("la-mia-app"));
        assert_eq!(package_name("città_ok 2").as_deref(), Some("citta_ok-2"));
        assert_eq!(package_name("  --demo--  ").as_deref(), Some("demo"));
        assert_eq!(package_name("3d viewer").as_deref(), Some("app-3d-viewer"));
        assert_eq!(package_name("test").as_deref(), Some("app-test"));
        assert_eq!(package_name("my.app").as_deref(), Some("my-app"));
        assert_eq!(package_name("日本"), None);
        assert_eq!(package_name(""), None);
    }

    #[test]
    fn names_are_validated() {
        let rust = ProjectTemplate::RustSlint;
        assert_eq!(validate_name("Contatore", rust), Ok(()));
        assert_eq!(validate_name("   ", rust), Err(NameError::Empty));
        assert_eq!(
            validate_name("a/b", rust),
            Err(NameError::InvalidCharacter('/'))
        );
        assert_eq!(
            validate_name("a:b", rust),
            Err(NameError::InvalidCharacter(':'))
        );
        assert_eq!(validate_name(".hidden", rust), Err(NameError::Reserved));
        assert_eq!(validate_name("..", rust), Err(NameError::Reserved));
        assert_eq!(validate_name("日本", rust), Err(NameError::NoPackageName));
        // Only Rust projects need a package name.
        assert_eq!(validate_name("日本", ProjectTemplate::SlintOnly), Ok(()));
        assert!(!NameError::Empty.to_string().is_empty());
    }

    #[test]
    fn rust_template_files() {
        let files = files(ProjectTemplate::RustSlint, "La mia \"App\"");
        let paths: Vec<_> = files
            .iter()
            .map(|(p, _)| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            paths,
            [
                "Cargo.toml",
                "build.rs",
                "src/main.rs",
                MAIN_FILE,
                ".gitignore"
            ]
        );
        let cargo = &files[0].1;
        assert!(cargo.contains("name = \"la-mia-app\""), "{cargo}");
        assert!(cargo.contains(&format!("slint = \"{SLINT_VERSION}\"")));
        assert!(cargo.contains(&format!("slint-build = \"{SLINT_VERSION}\"")));
        assert!(
            files[1]
                .1
                .contains("slint_build::compile(\"ui/app-window.slint\")")
        );
        assert!(files[2].1.contains("on_request_increase_value"));
        let ui = &files[3].1;
        assert!(ui.contains(r#"title: "La mia \"App\"";"#), "{ui}");
        assert!(
            ui.contains(r#"text: "Contatore: \{root.counter}";"#),
            "{ui}"
        );
        assert!(ui.contains("callback request-increase-value();"));
    }

    #[test]
    fn slint_only_template_has_a_single_file() {
        let files = files(ProjectTemplate::SlintOnly, "Bozza");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, Path::new(MAIN_FILE));
        assert!(files[0].1.contains("root.counter += 1;"));
        assert!(files[0].1.contains("title: \"Bozza\";"));
    }

    #[test]
    fn create_writes_the_project() {
        let parent = tempfile::tempdir().unwrap();
        let dir = create(parent.path(), " Contatore ", ProjectTemplate::RustSlint).unwrap();
        assert_eq!(dir, parent.path().join("Contatore"));
        for file in [
            "Cargo.toml",
            "build.rs",
            "src/main.rs",
            MAIN_FILE,
            ".gitignore",
        ] {
            assert!(dir.join(file).is_file(), "{file} missing");
        }

        // An existing, non-empty folder is never overwritten.
        let err = create(parent.path(), "Contatore", ProjectTemplate::SlintOnly).unwrap_err();
        assert!(matches!(err, CreateError::NotEmpty(_)), "{err}");
        assert!(err.to_string().contains("non è vuota"));

        // An existing empty folder is fine.
        fs::create_dir(parent.path().join("vuota")).unwrap();
        let dir = create(parent.path(), "vuota", ProjectTemplate::SlintOnly).unwrap();
        assert!(dir.join(MAIN_FILE).is_file());
    }

    #[test]
    fn invalid_locations_are_refused() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join("missing");
        assert!(matches!(
            target_dir(&missing, "app", ProjectTemplate::RustSlint),
            Err(CreateError::NoParent(_))
        ));
        assert!(matches!(
            target_dir(parent.path(), "", ProjectTemplate::RustSlint),
            Err(CreateError::InvalidName(NameError::Empty))
        ));
        assert_eq!(
            target_dir(parent.path(), "app", ProjectTemplate::RustSlint).unwrap(),
            parent.path().join("app")
        );
    }
}
