//! Renders a `.slint` file to a PNG, as the live preview would.
//!
//! `cargo run -p slint-studio-preview --example render -- ui/main.slint out.png [scale]`

use std::path::PathBuf;
use std::process::ExitCode;

use slint_studio_preview::{PreviewRequest, Previewer};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("uso: render <file.slint> <output.png> [scale]");
        return ExitCode::from(2);
    };
    let mut request = PreviewRequest::new(PathBuf::from(input));
    request.scale_factor = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);

    let result = Previewer::new().and_then(|mut previewer| previewer.render(&request));
    match result {
        Ok(preview) => {
            println!(
                "{}: {}×{} logici, {}×{} pixel, {:?}",
                preview.component,
                preview.logical_size.0,
                preview.logical_size.1,
                preview.image.width(),
                preview.image.height(),
                preview.duration
            );
            match preview.image.save(&output) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("errore: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(err) => {
            eprintln!("errore: {err}");
            if let slint_studio_preview::PreviewError::Compile { diagnostics } = &err {
                for d in diagnostics {
                    let at = d
                        .location
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    eprintln!("  {at}: {}", d.message);
                }
            }
            ExitCode::FAILURE
        }
    }
}
