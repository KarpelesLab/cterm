//! Quick Open overlay for rapidly searching and opening tab templates
//!
//! Shows a VS Code-style overlay at the top of the window for filtering templates.

use cterm_app::config::StickyTabConfig;
use cterm_app::{template_type_indicator, QuickOpenMatcher, TemplateMatch};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSColor, NSControlTextEditingDelegate, NSFont, NSStackView, NSTextField, NSTextFieldDelegate,
    NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSInteger, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize,
    NSString,
};
use std::cell::{Cell, RefCell};

/// Height of the Quick Open overlay
pub const QUICK_OPEN_HEIGHT: f64 = 250.0;

/// Maximum number of results to display
const MAX_RESULTS: usize = 8;

/// Ivars for the Quick Open overlay
pub struct QuickOpenOverlayIvars {
    /// Search text field
    search_field: RefCell<Option<Retained<NSTextField>>>,
    /// Results container stack view
    results_container: RefCell<Option<Retained<NSStackView>>>,
    /// Currently filtered templates
    filtered: RefCell<Vec<TemplateMatch>>,
    /// All available templates
    templates: RefCell<Vec<StickyTabConfig>>,
    /// Currently selected index
    selected_index: Cell<usize>,
    /// Callback when a template is selected
    on_select: RefCell<Option<Box<dyn Fn(StickyTabConfig)>>>,
    /// Result row views for highlighting
    result_rows: RefCell<Vec<Retained<NSView>>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickOpenOverlay"]
    #[ivars = QuickOpenOverlayIvars]
    pub struct QuickOpenOverlay;

    unsafe impl NSObjectProtocol for QuickOpenOverlay {}

    // NSControlTextEditingDelegate is required by NSTextFieldDelegate
    unsafe impl NSControlTextEditingDelegate for QuickOpenOverlay {
        /// Intercept commands from the text field (Enter, Escape, arrow keys)
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn control_text_view_do_command_by_selector(
            &self,
            _control: &objc2_app_kit::NSControl,
            _text_view: &objc2_app_kit::NSTextView,
            command_selector: objc2::runtime::Sel,
        ) -> bool {
            let sel_name = command_selector.name().to_str().unwrap_or("");
            log::debug!("doCommandBySelector: {}", sel_name);

            match sel_name {
                "insertNewline:" => {
                    self.confirm_selection();
                    true // We handled it
                }
                "cancelOperation:" => {
                    self.hide();
                    true
                }
                "moveUp:" => {
                    self.select_previous();
                    true
                }
                "moveDown:" => {
                    self.select_next();
                    true
                }
                "insertTab:" => {
                    self.select_next();
                    true
                }
                "insertBacktab:" => {
                    self.select_previous();
                    true
                }
                _ => false, // Let the text field handle it
            }
        }
    }

    // NSTextFieldDelegate for text changes
    unsafe impl NSTextFieldDelegate for QuickOpenOverlay {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _notification: &objc2_foundation::NSNotification) {
            self.update_filter();
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn control_text_did_end_editing(&self, _notification: &objc2_foundation::NSNotification) {
            // Text field lost focus - dismiss the overlay
            self.hide();
        }
    }

    impl QuickOpenOverlay {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// Handle key events for navigation
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &objc2_app_kit::NSEvent) {
            let key_code = event.keyCode();

            match key_code {
                125 => self.select_next(),     // Down arrow
                126 => self.select_previous(), // Up arrow
                36 => self.confirm_selection(), // Enter/Return
                53 => self.hide(),             // Escape
                48 => self.select_next(),      // Tab
                _ => {
                    // Pass to super for normal text input
                    unsafe {
                        let _: () = msg_send![super(self), keyDown: event];
                    }
                }
            }
        }
    }
);

impl QuickOpenOverlay {
    /// Create a new Quick Open overlay
    pub fn new(
        mtm: MainThreadMarker,
        width: f64,
        templates: Vec<StickyTabConfig>,
    ) -> Retained<Self> {
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, QUICK_OPEN_HEIGHT),
        );

        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(QuickOpenOverlayIvars {
            search_field: RefCell::new(None),
            results_container: RefCell::new(None),
            filtered: RefCell::new(Vec::new()),
            templates: RefCell::new(templates),
            selected_index: Cell::new(0),
            on_select: RefCell::new(None),
            result_rows: RefCell::new(Vec::new()),
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Set up appearance
        this.setWantsLayer(true);
        if let Some(layer) = this.layer() {
            unsafe {
                // Dark semi-transparent background
                let color = NSColor::colorWithSRGBRed_green_blue_alpha(0.12, 0.12, 0.12, 0.95);
                let cg_color: *mut std::ffi::c_void = msg_send![&*color, CGColor];
                let _: () = msg_send![&*layer, setBackgroundColor: cg_color];
                // Rounded corners
                let _: () = msg_send![&*layer, setCornerRadius: 8.0f64];
            }
        }

        // Create UI elements
        this.setup_ui(mtm, width);

        // Initially hidden
        this.setHidden(true);

        // Initialize with all templates
        this.update_filter();

        this
    }

    fn setup_ui(&self, mtm: MainThreadMarker, width: f64) {
        let padding = 12.0;
        let search_height = 28.0;
        let search_width = width - padding * 2.0;

        // Create search field
        let search_frame = NSRect::new(
            NSPoint::new(padding, padding),
            NSSize::new(search_width, search_height),
        );
        let search_field = unsafe { NSTextField::initWithFrame(mtm.alloc(), search_frame) };
        search_field.setPlaceholderString(Some(&NSString::from_str("Search templates...")));
        search_field.setBezeled(true);
        search_field.setEditable(true);
        search_field.setSelectable(true);

        // Style the search field
        unsafe {
            let font = NSFont::systemFontOfSize(14.0);
            search_field.setFont(Some(&font));
            search_field.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(self)));
        }

        unsafe {
            self.addSubview(&search_field);
        }
        *self.ivars().search_field.borrow_mut() = Some(search_field);

        // Create results container
        let results_y = padding + search_height + 8.0;
        let results_height = QUICK_OPEN_HEIGHT - results_y - padding;
        let results_frame = NSRect::new(
            NSPoint::new(padding, results_y),
            NSSize::new(search_width, results_height),
        );

        let results_container = unsafe {
            let stack = NSStackView::initWithFrame(mtm.alloc(), results_frame);
            stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
            stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
            stack.setSpacing(2.0);
            // Use GravityAreas distribution to prevent rows from stretching to fill
            stack.setDistribution(objc2_app_kit::NSStackViewDistribution::GravityAreas);
            stack
        };

        unsafe {
            self.addSubview(&results_container);
        }
        *self.ivars().results_container.borrow_mut() = Some(results_container);
    }

    /// Show the overlay and focus the search field
    pub fn show(&self) {
        self.setHidden(false);

        // Clear search field and reset selection
        if let Some(ref search_field) = *self.ivars().search_field.borrow() {
            search_field.setStringValue(&NSString::from_str(""));
            unsafe {
                self.window()
                    .map(|w| w.makeFirstResponder(Some(search_field)));
            }
        }

        // Reset filter to show all templates
        self.ivars().selected_index.set(0);
        self.update_filter();
    }

    /// Hide the overlay
    pub fn hide(&self) {
        self.setHidden(true);

        // Return focus to the window's content view (terminal)
        unsafe {
            if let Some(window) = self.window() {
                if let Some(content) = window.contentView() {
                    window.makeFirstResponder(Some(&content));
                }
            }
        }
    }

    /// Check if overlay is visible
    pub fn is_visible(&self) -> bool {
        !self.is_hidden()
    }

    fn is_hidden(&self) -> bool {
        unsafe { msg_send![self, isHidden] }
    }

    /// Set the callback for template selection
    pub fn set_on_select<F>(&self, callback: F)
    where
        F: Fn(StickyTabConfig) + 'static,
    {
        *self.ivars().on_select.borrow_mut() = Some(Box::new(callback));
    }

    /// Update the filter based on current search text
    fn update_filter(&self) {
        let query = self
            .ivars()
            .search_field
            .borrow()
            .as_ref()
            .map(|f| f.stringValue().to_string())
            .unwrap_or_default();

        let templates = self.ivars().templates.borrow().clone();
        let matcher = QuickOpenMatcher::new(templates);
        let filtered = matcher.filter(&query);

        // Limit to MAX_RESULTS
        let filtered: Vec<TemplateMatch> = filtered.into_iter().take(MAX_RESULTS).collect();

        *self.ivars().filtered.borrow_mut() = filtered;

        // Reset selection to first item
        self.ivars().selected_index.set(0);

        // Rebuild result rows
        self.rebuild_results();
    }

    /// Rebuild the results UI
    fn rebuild_results(&self) {
        let mtm = MainThreadMarker::from(self);

        // Clear existing rows
        if let Some(ref container) = *self.ivars().results_container.borrow() {
            // Remove all arranged subviews
            for view in self.ivars().result_rows.borrow().iter() {
                container.removeArrangedSubview(view);
                view.removeFromSuperview();
            }
        }
        self.ivars().result_rows.borrow_mut().clear();

        let filtered = self.ivars().filtered.borrow();
        let selected_idx = self.ivars().selected_index.get();

        if filtered.is_empty() {
            // Show "No templates" message
            if let Some(ref container) = *self.ivars().results_container.borrow() {
                let label = self.create_result_row(mtm, "No templates found", "", false);
                container.addArrangedSubview(&label);
                self.ivars().result_rows.borrow_mut().push(label);
            }
            return;
        }

        if let Some(ref container) = *self.ivars().results_container.borrow() {
            for (idx, match_result) in filtered.iter().enumerate() {
                let indicator = template_type_indicator(&match_result.template);
                let row = self.create_result_row(
                    mtm,
                    &match_result.template.name,
                    indicator,
                    idx == selected_idx,
                );
                container.addArrangedSubview(&row);
                self.ivars().result_rows.borrow_mut().push(row);
            }
        }
    }

    /// Create a result row view
    fn create_result_row(
        &self,
        mtm: MainThreadMarker,
        name: &str,
        indicator: &str,
        selected: bool,
    ) -> Retained<NSView> {
        let container_width = self.frame().size.width - 24.0; // Account for padding
        let row_height = 24.0;
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(container_width, row_height));

        let row: Retained<NSView> = unsafe {
            let view = NSView::initWithFrame(mtm.alloc(), frame);
            view.setWantsLayer(true);
            view
        };

        // Set background color based on selection
        if let Some(layer) = row.layer() {
            unsafe {
                let color = if selected {
                    NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.4, 0.8, 1.0)
                } else {
                    NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 0.0)
                };
                let cg_color: *mut std::ffi::c_void = msg_send![&*color, CGColor];
                let _: () = msg_send![&*layer, setBackgroundColor: cg_color];
                let _: () = msg_send![&*layer, setCornerRadius: 4.0f64];
            }
        }

        // Create label for name
        let label_frame = NSRect::new(
            NSPoint::new(8.0, 2.0),
            NSSize::new(container_width - 40.0, row_height - 4.0),
        );
        let label = unsafe { NSTextField::initWithFrame(mtm.alloc(), label_frame) };
        label.setStringValue(&NSString::from_str(name));
        label.setBezeled(false);
        label.setDrawsBackground(false);
        label.setEditable(false);
        label.setSelectable(false);
        unsafe {
            let white = NSColor::whiteColor();
            label.setTextColor(Some(&white));
            let font = NSFont::systemFontOfSize(13.0);
            label.setFont(Some(&font));
            row.addSubview(&label);
        }

        // Create indicator label (right side)
        if !indicator.is_empty() {
            let indicator_frame = NSRect::new(
                NSPoint::new(container_width - 30.0, 2.0),
                NSSize::new(24.0, row_height - 4.0),
            );
            let indicator_label =
                unsafe { NSTextField::initWithFrame(mtm.alloc(), indicator_frame) };
            indicator_label.setStringValue(&NSString::from_str(indicator));
            indicator_label.setBezeled(false);
            indicator_label.setDrawsBackground(false);
            indicator_label.setEditable(false);
            indicator_label.setSelectable(false);
            unsafe {
                row.addSubview(&indicator_label);
            }
        }

        // Set fixed width constraint
        unsafe {
            let width_constraint: *mut AnyObject = msg_send![
                objc2::class!(NSLayoutConstraint),
                constraintWithItem: &*row,
                attribute: 7i64,  // NSLayoutAttributeWidth
                relatedBy: 0i64,  // NSLayoutRelationEqual
                toItem: std::ptr::null::<AnyObject>(),
                attribute: 0i64,  // NSLayoutAttributeNotAnAttribute
                multiplier: 1.0f64,
                constant: container_width
            ];
            let _: () = msg_send![width_constraint, setActive: true];
        }

        row
    }

    /// Select the next item in the list
    fn select_next(&self) {
        let filtered = self.ivars().filtered.borrow();
        if filtered.is_empty() {
            return;
        }

        let current = self.ivars().selected_index.get();
        let new_index = if current + 1 >= filtered.len() {
            0 // Wrap around
        } else {
            current + 1
        };
        drop(filtered);

        self.ivars().selected_index.set(new_index);
        self.update_selection_highlight();
    }

    /// Select the previous item in the list
    fn select_previous(&self) {
        let filtered = self.ivars().filtered.borrow();
        if filtered.is_empty() {
            return;
        }

        let current = self.ivars().selected_index.get();
        let new_index = if current == 0 {
            filtered.len() - 1 // Wrap around
        } else {
            current - 1
        };
        drop(filtered);

        self.ivars().selected_index.set(new_index);
        self.update_selection_highlight();
    }

    /// Update the visual highlight for selection
    fn update_selection_highlight(&self) {
        let selected_idx = self.ivars().selected_index.get();
        let rows = self.ivars().result_rows.borrow();

        for (idx, row) in rows.iter().enumerate() {
            if let Some(layer) = row.layer() {
                unsafe {
                    let color = if idx == selected_idx {
                        NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.4, 0.8, 1.0)
                    } else {
                        NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 0.0)
                    };
                    let cg_color: *mut std::ffi::c_void = msg_send![&*color, CGColor];
                    let _: () = msg_send![&*layer, setBackgroundColor: cg_color];
                }
            }
        }
    }

    /// Confirm the current selection
    fn confirm_selection(&self) {
        let filtered = self.ivars().filtered.borrow();
        let selected_idx = self.ivars().selected_index.get();

        if let Some(match_result) = filtered.get(selected_idx) {
            let template = match_result.template.clone();
            drop(filtered);

            // Hide first
            self.hide();

            // Call callback
            if let Some(ref callback) = *self.ivars().on_select.borrow() {
                callback(template);
            }
        }
    }

    /// Update templates list
    pub fn set_templates(&self, templates: Vec<StickyTabConfig>) {
        *self.ivars().templates.borrow_mut() = templates;
        self.update_filter();
    }

    /// Update the overlay width when window resizes
    pub fn update_width(&self, width: f64) {
        let mut frame = self.frame();
        frame.size.width = width;
        self.setFrame(frame);

        // Update search field width
        let padding = 12.0;
        let search_width = width - padding * 2.0;

        if let Some(ref search_field) = *self.ivars().search_field.borrow() {
            let mut search_frame = search_field.frame();
            search_frame.size.width = search_width;
            search_field.setFrame(search_frame);
        }

        if let Some(ref results_container) = *self.ivars().results_container.borrow() {
            let mut results_frame = results_container.frame();
            results_frame.size.width = search_width;
            unsafe {
                let _: () = msg_send![&**results_container, setFrame: results_frame];
            }
        }

        // Rebuild results to update row widths
        self.rebuild_results();
    }

    // Helper methods
    fn frame(&self) -> NSRect {
        unsafe { msg_send![self, frame] }
    }

    #[allow(non_snake_case)]
    fn setFrame(&self, frame: NSRect) {
        unsafe {
            let _: () = msg_send![self, setFrame: frame];
        }
    }

    #[allow(non_snake_case)]
    fn setHidden(&self, hidden: bool) {
        unsafe {
            let _: () = msg_send![self, setHidden: hidden];
        }
    }

    #[allow(non_snake_case)]
    fn setWantsLayer(&self, wants: bool) {
        unsafe {
            let _: () = msg_send![self, setWantsLayer: wants];
        }
    }

    fn layer(&self) -> Option<Retained<AnyObject>> {
        unsafe { msg_send![self, layer] }
    }

    fn window(&self) -> Option<Retained<objc2_app_kit::NSWindow>> {
        unsafe { msg_send![self, window] }
    }
}
