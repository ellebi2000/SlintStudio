//! A headless Slint window and platform backed by the software renderer.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use slint::platform::software_renderer::{RepaintBufferType, SoftwareRenderer};
use slint::platform::{
    Platform, PlatformError, Renderer, WindowAdapter, WindowEvent, WindowProperties,
};
use slint::{LogicalSize, PhysicalSize, Window, WindowSize};

/// A window that is never shown on screen: it only renders into buffers.
pub(crate) struct PreviewWindow {
    window: Window,
    renderer: SoftwareRenderer,
    size: Cell<PhysicalSize>,
    /// Preferred size of the root component, reported by Slint when the
    /// window is shown.
    preferred: Cell<Option<LogicalSize>>,
}

impl PreviewWindow {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new_cyclic(|weak: &Weak<Self>| Self {
            window: Window::new(weak.clone()),
            renderer: SoftwareRenderer::new_with_repaint_buffer_type(RepaintBufferType::NewBuffer),
            size: Cell::default(),
            preferred: Cell::default(),
        })
    }

    pub(crate) fn window(&self) -> &Window {
        &self.window
    }

    pub(crate) fn software_renderer(&self) -> &SoftwareRenderer {
        &self.renderer
    }

    pub(crate) fn preferred_size(&self) -> Option<LogicalSize> {
        self.preferred.get()
    }
}

impl WindowAdapter for PreviewWindow {
    fn window(&self) -> &Window {
        &self.window
    }

    fn size(&self) -> PhysicalSize {
        self.size.get()
    }

    fn set_size(&self, size: WindowSize) {
        let scale_factor = self.window.scale_factor();
        self.size.set(size.to_physical(scale_factor));
        self.window.dispatch_event(WindowEvent::Resized {
            size: size.to_logical(scale_factor),
        });
    }

    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }

    fn update_window_properties(&self, properties: WindowProperties<'_>) {
        self.preferred
            .set(Some(properties.layout_constraints().preferred));
    }
}

thread_local! {
    /// Windows created by the platform on this thread since the last
    /// [`take_created_windows`] call.
    static CREATED: RefCell<Vec<Rc<PreviewWindow>>> = const { RefCell::new(Vec::new()) };
}

/// Returns (and forgets) the windows created on this thread, oldest first.
/// After `ComponentDefinition::create`, the first one is the component's.
pub(crate) fn take_created_windows() -> Vec<Rc<PreviewWindow>> {
    CREATED.with(|created| std::mem::take(&mut *created.borrow_mut()))
}

/// Minimal headless platform for the preview thread: every window Slint
/// asks for is a [`PreviewWindow`], recorded so the previewer can render it.
pub(crate) struct PreviewPlatform;

impl Platform for PreviewPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = PreviewWindow::new();
        CREATED.with(|created| created.borrow_mut().push(Rc::clone(&window)));
        Ok(window)
    }
}
