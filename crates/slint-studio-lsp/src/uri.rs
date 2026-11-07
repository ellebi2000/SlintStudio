//! Conversions between file paths and LSP `file://` URIs.

use std::path::{Path, PathBuf};

use lsp_types::Uri;

/// `file://` URI of an absolute path (`None` for relative paths).
pub fn uri_from_path(path: &Path) -> Option<Uri> {
    let url = url::Url::from_file_path(path).ok()?;
    url.as_str().parse().ok()
}

/// Local path of a `file://` URI (`None` for other schemes, e.g. `builtin:`).
pub fn path_from_uri(uri: &Uri) -> Option<PathBuf> {
    let url = url::Url::parse(uri.as_str()).ok()?;
    // `to_file_path` does not check the scheme: `builtin:/x.slint` would
    // otherwise become the local path `/x.slint`.
    if url.scheme() != "file" {
        return None;
    }
    url.to_file_path().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn round_trip_with_special_characters() {
        let path = Path::new("/progetti/mia app/ui/perché #1.slint");
        let uri = uri_from_path(path).unwrap();
        assert_eq!(
            uri.as_str(),
            "file:///progetti/mia%20app/ui/perch%C3%A9%20%231.slint"
        );
        assert_eq!(path_from_uri(&uri).unwrap(), path);
    }

    #[cfg(windows)]
    #[test]
    fn round_trip_windows_path() {
        let path = Path::new(r"C:\progetti\app\main.slint");
        let uri = uri_from_path(path).unwrap();
        assert_eq!(uri.as_str(), "file:///C:/progetti/app/main.slint");
        assert_eq!(path_from_uri(&uri).unwrap(), path);
    }

    #[test]
    fn relative_paths_and_other_schemes_are_rejected() {
        assert!(uri_from_path(Path::new("ui/main.slint")).is_none());
        let builtin: Uri = "builtin:/fluent/std-widgets.slint".parse().unwrap();
        assert!(path_from_uri(&builtin).is_none());
    }
}
