//! End-to-end test against a real `slint-lsp`.
//!
//! Runs only when `SLINT_STUDIO_LSP` points to the executable, e.g.
//! `SLINT_STUDIO_LSP=~/.local/share/slint-studio/slint-lsp/1.18.1/slint-lsp cargo test -p slint-studio-lsp`.

// Test helpers may panic: a failed expectation is a failed test.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use slint_studio_core::{LineCol, LineIndex, PositionEncoding};
use slint_studio_lsp::{LspClient, LspEvent, ServerConfig};

const SOURCE: &str = r#"import { Button } from "std-widgets.slint";

component Card inherits Rectangle {
    in property <string> title;
    Text { text: root.title; }
}

export component MainWindow inherits Window {
    Card { title: "Ciao"; }
    Button { text: "OK"; }
    Rectangle { backg }
}
"#;

fn next_event(events: &Receiver<LspEvent>, mut accept: impl FnMut(&LspEvent) -> bool) -> LspEvent {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = events
            .recv_timeout(remaining)
            .expect("timed out waiting for slint-lsp");
        if let LspEvent::Exited { code, stderr_tail } = &event {
            panic!("slint-lsp exited ({code:?}): {stderr_tail:?}");
        }
        if accept(&event) {
            return event;
        }
    }
}

fn position_of(text: &str, needle: &str, offset: usize, encoding: PositionEncoding) -> LineCol {
    let byte = text.find(needle).expect("needle") + offset;
    LineIndex::new(text).position(text, byte, encoding)
}

#[test]
fn diagnostics_completion_and_definition() {
    let Some(binary) = std::env::var_os("SLINT_STUDIO_LSP") else {
        eprintln!("SLINT_STUDIO_LSP not set: skipping the slint-lsp end-to-end test");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let file = root.join("main.slint");
    std::fs::write(&file, SOURCE).unwrap();

    let config = ServerConfig {
        binary: binary.into(),
        args: Vec::new(),
        root: root.clone(),
    };
    let (client, events) = LspClient::start(&config, Arc::new(|| {})).unwrap();

    let LspEvent::Ready { server, encoding } =
        next_event(&events, |e| matches!(e, LspEvent::Ready { .. }))
    else {
        unreachable!()
    };
    assert!(server.contains("slint-lsp"), "{server}");
    assert!(client.is_ready());

    client.did_open(&file, SOURCE, 1);
    let LspEvent::Diagnostics {
        path, diagnostics, ..
    } = next_event(&events, |e| matches!(e, LspEvent::Diagnostics { .. }))
    else {
        unreachable!()
    };
    assert_eq!(path, file);
    assert!(
        !diagnostics.is_empty(),
        "the incomplete `backg` binding must be reported"
    );

    let at = position_of(SOURCE, "backg", 5, encoding);
    let request = client.completion(&file, at).unwrap();
    let LspEvent::Completion { items, .. } = next_event(
        &events,
        |e| matches!(e, LspEvent::Completion { request: r, .. } if *r == request),
    ) else {
        unreachable!()
    };
    assert!(items.iter().any(|i| i.label == "background"), "{items:?}");

    let at = position_of(SOURCE, "Card { title", 1, encoding);
    let request = client.definition(&file, at).unwrap();
    let LspEvent::Definition { targets, .. } = next_event(
        &events,
        |e| matches!(e, LspEvent::Definition { request: r, .. } if *r == request),
    ) else {
        unreachable!()
    };
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].path.as_deref(), Some(file.as_path()));
    assert_eq!(
        targets[0].position.line, 2,
        "definition of Card is on line 3"
    );

    client.shutdown();
}
