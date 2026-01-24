//! Log viewer window for debugging
//!
//! Displays captured application logs in a scrollable text view.

use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSScrollView, NSTextView, NSWindow, NSWindowStyleMask};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::log_capture;

/// Log viewer window state
pub struct LogViewerWindowIvars {
    text_view: std::cell::RefCell<Option<Retained<NSTextView>>>,
}

define_class!(
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "LogViewerWindow"]
    #[ivars = LogViewerWindowIvars]
    pub struct LogViewerWindow;

    unsafe impl NSObjectProtocol for LogViewerWindow {}
);

impl LogViewerWindow {
    /// Create a new log viewer window
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let content_rect = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(800.0, 600.0));

        let style_mask = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Resizable
            | NSWindowStyleMask::Miniaturizable;

        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(LogViewerWindowIvars {
            text_view: std::cell::RefCell::new(None),
        });

        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: content_rect,
                styleMask: style_mask,
                backing: 2u64, // NSBackingStoreBuffered
                defer: false
            ]
        };

        this.setTitle(&NSString::from_str("Application Logs"));
        this.setMinSize(NSSize::new(400.0, 300.0));

        // Prevent double-free when window closes
        unsafe { this.setReleasedWhenClosed(false) };

        // Build the UI
        this.build_ui(mtm);

        // Load initial logs
        this.refresh_logs();

        this
    }

    fn build_ui(&self, mtm: MainThreadMarker) {
        // Create scroll view
        let scroll_view = unsafe {
            let frame = NSRect::new(NSPoint::ZERO, NSSize::new(800.0, 600.0));
            let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
            scroll.setHasVerticalScroller(true);
            scroll.setHasHorizontalScroller(true);
            scroll.setAutohidesScrollers(false);
            scroll
        };

        // Create text view
        let text_view = unsafe {
            let frame = NSRect::new(NSPoint::ZERO, NSSize::new(800.0, 600.0));
            let text = NSTextView::initWithFrame(NSTextView::alloc(mtm), frame);

            // Make it read-only and use monospace font
            text.setEditable(false);
            text.setSelectable(true);

            // Use monospace font
            let font: *mut objc2::runtime::AnyObject = msg_send![
                objc2::class!(NSFont),
                monospacedSystemFontOfSize: 11.0f64,
                weight: 0.0f64  // Regular weight
            ];
            let _: () = msg_send![&*text, setFont: font];

            // Set background color to dark
            let bg_color: *mut objc2::runtime::AnyObject = msg_send![
                objc2::class!(NSColor),
                colorWithRed: 0.1f64,
                green: 0.1f64,
                blue: 0.1f64,
                alpha: 1.0f64
            ];
            let _: () = msg_send![&*text, setBackgroundColor: bg_color];

            // Set text color to light
            let text_color: *mut objc2::runtime::AnyObject = msg_send![
                objc2::class!(NSColor),
                colorWithRed: 0.9f64,
                green: 0.9f64,
                blue: 0.9f64,
                alpha: 1.0f64
            ];
            let _: () = msg_send![&*text, setTextColor: text_color];

            // Allow horizontal scrolling
            let _: () = msg_send![&*text, setHorizontallyResizable: true];
            let max_size = NSSize::new(f64::MAX, f64::MAX);
            let _: () = msg_send![&*text, setMaxSize: max_size];

            // Get text container and configure it
            let container: *mut objc2::runtime::AnyObject = msg_send![&*text, textContainer];
            if !container.is_null() {
                let _: () = msg_send![container, setWidthTracksTextView: false];
                let large_size = NSSize::new(1000000.0, 1000000.0);
                let _: () = msg_send![container, setContainerSize: large_size];
            }

            text
        };

        // Set text view as document view
        scroll_view.setDocumentView(Some(&text_view));

        // Store reference
        *self.ivars().text_view.borrow_mut() = Some(text_view);

        // Set as content view
        self.setContentView(Some(&scroll_view));
    }

    /// Refresh the log display
    pub fn refresh_logs(&self) {
        let logs = log_capture::get_logs_formatted();

        if let Some(text_view) = self.ivars().text_view.borrow().as_ref() {
            let ns_string = NSString::from_str(&logs);
            text_view.setString(&ns_string);

            // Scroll to bottom
            unsafe {
                let length: usize = msg_send![&*ns_string, length];
                let range = objc2_foundation::NSRange::new(length, 0);
                let _: () = msg_send![&**text_view, scrollRangeToVisible: range];
            }
        }
    }
}

/// Show the log viewer window
pub fn show_log_viewer(mtm: MainThreadMarker) -> Retained<LogViewerWindow> {
    let window = LogViewerWindow::new(mtm);
    window.makeKeyAndOrderFront(None);
    window
}
