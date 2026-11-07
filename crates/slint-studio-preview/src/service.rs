//! Rendering on a dedicated thread, for use from the UI thread.

use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use tracing::{debug, warn};

use crate::renderer::{PreviewError, PreviewRequest, Previewer, RenderedPreview};

/// Called after each response is queued; typically requests a UI repaint.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Result of one request, tagged with the generation returned by
/// [`PreviewService::request`].
#[derive(Debug)]
pub struct PreviewResponse {
    pub generation: u64,
    pub result: Result<RenderedPreview, PreviewError>,
}

struct Job {
    generation: u64,
    request: PreviewRequest,
}

/// Owns the preview thread. Requests are coalesced: when several are queued,
/// only the most recent one is rendered.
pub struct PreviewService {
    jobs: Sender<Job>,
    responses: Receiver<PreviewResponse>,
    generation: u64,
}

impl PreviewService {
    pub fn start(waker: Waker) -> io::Result<Self> {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (response_tx, responses) = mpsc::channel();
        thread::Builder::new()
            .name("slint-preview".into())
            .spawn(move || worker(&job_rx, &response_tx, &waker))?;
        Ok(Self {
            jobs,
            responses,
            generation: 0,
        })
    }

    /// Queues a render; returns its generation (increasing).
    pub fn request(&mut self, request: PreviewRequest) -> u64 {
        self.generation += 1;
        let job = Job {
            generation: self.generation,
            request,
        };
        if self.jobs.send(job).is_err() {
            warn!("il thread dell'anteprima non è più attivo");
        }
        self.generation
    }

    /// Generation of the most recent request.
    pub fn latest_generation(&self) -> u64 {
        self.generation
    }

    /// The newest response received since the last call, if any.
    pub fn poll(&self) -> Option<PreviewResponse> {
        self.responses.try_iter().max_by_key(|r| r.generation)
    }
}

fn worker(jobs: &Receiver<Job>, responses: &Sender<PreviewResponse>, waker: &Waker) {
    let mut previewer = match Previewer::new() {
        Ok(previewer) => Some(previewer),
        Err(err) => {
            warn!("anteprima non disponibile: {err}");
            None
        }
    };
    while let Ok(mut job) = jobs.recv() {
        // Coalesce: skip requests superseded while we were busy.
        while let Ok(newer) = jobs.try_recv() {
            job = newer;
        }
        debug!(generation = job.generation, path = %job.request.path.display(), "rendering anteprima");
        let result = match &mut previewer {
            Some(previewer) => {
                panic::catch_unwind(AssertUnwindSafe(|| previewer.render(&job.request)))
                    .unwrap_or_else(|payload| {
                        Err(PreviewError::Internal(panic_message(payload.as_ref())))
                    })
            }
            None => Err(PreviewError::Platform(
                "inizializzazione non riuscita".into(),
            )),
        };
        let response = PreviewResponse {
            generation: job.generation,
            result,
        };
        if responses.send(response).is_err() {
            return; // the service was dropped
        }
        waker();
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic senza messaggio".to_owned())
}
