//! Rendering real `.slint` sources with the software renderer.

// Test helpers may panic: a failed expectation is a failed test.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use slint_studio_core::{LineCol, Severity};
use slint_studio_preview::{PreviewError, PreviewRequest, PreviewService, Previewer, render};

fn write(dir: &Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, source).unwrap();
    path.canonicalize().unwrap()
}

const RED_100X50: &str = "export component Red inherits Window {
    width: 100px;
    height: 50px;
    background: #ff0000;
}";

#[test]
fn renders_a_coloured_window_at_its_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "red.slint", RED_100X50);
    let image = render(&path).unwrap();
    assert_eq!(image.dimensions(), (100, 50));
    assert_eq!(image.get_pixel(50, 25).0, [255, 0, 0, 255]);
}

#[test]
fn scale_factor_multiplies_the_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "red.slint", RED_100X50);
    let mut request = PreviewRequest::new(&path);
    request.scale_factor = 2.0;
    let preview = Previewer::new().unwrap().render(&request).unwrap();
    assert_eq!(preview.image.dimensions(), (200, 100));
    assert_eq!(preview.logical_size, (100.0, 50.0));
    assert_eq!(preview.component, "Red");
}

#[test]
fn layouts_give_the_preferred_size_and_text_is_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "text.slint",
        "export component Label inherits Window {
            background: white;
            VerticalLayout {
                padding: 10px;
                Text { text: \"Ciao Slint\"; color: black; font-size: 20px; }
            }
        }",
    );
    let image = render(&path).unwrap();
    let (width, height) = image.dimensions();
    assert!(width > 20 && height > 20, "preferred size {width}×{height}");
    let dark = image
        .pixels()
        .filter(|p| p.0[0] < 100 && p.0[3] == 255)
        .count();
    assert!(dark > 20, "text pixels: {dark}");
}

#[test]
fn std_widgets_render() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "widgets.slint",
        "import { Button, VerticalBox } from \"std-widgets.slint\";
        export component W inherits Window {
            VerticalBox { Button { text: \"OK\"; } }
        }",
    );
    let image = render(&path).unwrap();
    assert!(image.width() > 10 && image.height() > 10);
}

#[test]
fn unsaved_buffers_override_the_disk_including_imports() {
    let dir = tempfile::tempdir().unwrap();
    let swatch = write(
        dir.path(),
        "parts/swatch.slint",
        "export component Swatch inherits Rectangle { width: 40px; height: 40px; background: #ff0000; }",
    );
    let main = write(
        dir.path(),
        "main.slint",
        "import { Swatch } from \"parts/swatch.slint\";
        export component Main inherits Window { width: 40px; height: 40px; Swatch { } }",
    );
    assert_eq!(render(&main).unwrap().get_pixel(20, 20).0, [255, 0, 0, 255]);

    let mut request = PreviewRequest::new(&main);
    request.overrides.insert(
        swatch.clone(),
        "export component Swatch inherits Rectangle { width: 40px; height: 40px; background: #0000ff; }".into(),
    );
    let preview = Previewer::new().unwrap().render(&request).unwrap();
    assert_eq!(preview.image.get_pixel(20, 20).0, [0, 0, 255, 255]);

    request.overrides.insert(
        main.clone(),
        "export component Main inherits Window { width: 40px; height: 40px; background: #00ff00; }"
            .into(),
    );
    let preview = Previewer::new().unwrap().render(&request).unwrap();
    assert_eq!(preview.image.get_pixel(20, 20).0, [0, 255, 0, 255]);
}

#[test]
fn compile_errors_are_reported_with_character_positions() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "bad.slint",
        "// è\nexport component Bad inherits Window {\n    Text { colour: red; }\n}\n",
    );
    let Err(PreviewError::Compile { diagnostics }) = Previewer::new()
        .unwrap()
        .render(&PreviewRequest::new(&path))
    else {
        panic!("expected a compile error");
    };
    let error = diagnostics
        .iter()
        .find(|d| d.severity == Severity::Error)
        .unwrap();
    assert!(error.message.contains("colour"), "{}", error.message);
    let location = error.location.as_ref().unwrap();
    assert_eq!(location.path, path);
    assert_eq!(location.start, LineCol::new(2, 11));
    assert!(location.end.column > location.start.column);
}

#[test]
fn files_without_components_and_missing_files_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "types.slint",
        "export struct Point { x: int, y: int }",
    );
    assert!(matches!(render(&path), Err(PreviewError::NoComponent)));
    assert!(matches!(
        render(&dir.path().join("missing.slint")),
        Err(PreviewError::Read { .. })
    ));
}

#[test]
fn service_renders_on_its_own_thread_and_coalesces() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "red.slint", RED_100X50);
    let mut service = PreviewService::start(Arc::new(|| {})).unwrap();
    for _ in 0..5 {
        service.request(PreviewRequest::new(&path));
    }
    let latest = service.latest_generation();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(response) = service.poll()
            && response.generation == latest
        {
            assert_eq!(response.result.unwrap().image.dimensions(), (100, 50));
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no response from the preview thread"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
