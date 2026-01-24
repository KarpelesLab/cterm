//! Notification bar for file transfer UI
//!
//! Shows a dismissible notification when files are received via iTerm2 protocol.
//! Format: "Received file: Name.bin (1.2 MB)" [Save] [Save As...] [Discard]

use cterm_ui::format_size;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSBezelStyle, NSButton, NSButtonType, NSColor, NSTextField, NSView};
use objc2_foundation::{
    MainThreadMarker, NSAttributedString, NSDictionary, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString,
};
use std::cell::{Cell, RefCell};

/// Notification bar height in pixels
pub const NOTIFICATION_BAR_HEIGHT: f64 = 32.0;

/// Ivars for the notification bar
pub struct NotificationBarIvars {
    /// File ID for current notification
    file_id: Cell<u64>,
    /// Label showing file name and size
    label: RefCell<Option<Retained<NSTextField>>>,
    /// Save button
    save_button: RefCell<Option<Retained<NSButton>>>,
    /// Save As button
    save_as_button: RefCell<Option<Retained<NSButton>>>,
    /// Discard button
    discard_button: RefCell<Option<Retained<NSButton>>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "NotificationBar"]
    #[ivars = NotificationBarIvars]
    pub struct NotificationBar;

    unsafe impl NSObjectProtocol for NotificationBar {}

    impl NotificationBar {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl NotificationBar {
    /// Create a new notification bar
    pub fn new(mtm: MainThreadMarker, width: f64) -> Retained<Self> {
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, NOTIFICATION_BAR_HEIGHT),
        );

        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(NotificationBarIvars {
            file_id: Cell::new(0),
            label: RefCell::new(None),
            save_button: RefCell::new(None),
            save_as_button: RefCell::new(None),
            discard_button: RefCell::new(None),
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Set background color
        this.setWantsLayer(true);
        if let Some(layer) = this.layer() {
            unsafe {
                let color = NSColor::colorWithSRGBRed_green_blue_alpha(0.2, 0.2, 0.2, 0.95);
                let cg_color: *mut std::ffi::c_void = msg_send![&*color, CGColor];
                let _: () = msg_send![&*layer, setBackgroundColor: cg_color];
            }
        }

        // Create and position UI elements
        this.setup_ui(mtm, width);

        // Initially hidden
        this.setHidden(true);

        this
    }

    fn setup_ui(&self, mtm: MainThreadMarker, width: f64) {
        let padding = 8.0;
        let button_width = 70.0;
        let button_height = 22.0;
        let button_spacing = 8.0;

        // Calculate positions from right edge
        let discard_x = width - padding - button_width;
        let save_as_x = discard_x - button_spacing - button_width;
        let save_x = save_as_x - button_spacing - button_width;
        let label_width = save_x - padding - padding;

        // Create label
        let label_frame = NSRect::new(
            NSPoint::new(padding, (NOTIFICATION_BAR_HEIGHT - button_height) / 2.0),
            NSSize::new(label_width, button_height),
        );
        let label = unsafe { NSTextField::initWithFrame(mtm.alloc(), label_frame) };
        label.setBezeled(false);
        label.setDrawsBackground(false);
        label.setEditable(false);
        label.setSelectable(false);
        unsafe {
            let white = NSColor::whiteColor();
            label.setTextColor(Some(&white));
        }
        unsafe {
            self.addSubview(&label);
        }
        *self.ivars().label.borrow_mut() = Some(label);

        // Create Save button
        let save_frame = NSRect::new(
            NSPoint::new(save_x, (NOTIFICATION_BAR_HEIGHT - button_height) / 2.0),
            NSSize::new(button_width, button_height),
        );
        let save_button = unsafe { NSButton::initWithFrame(mtm.alloc(), save_frame) };
        Self::configure_button(mtm, &save_button, "Save", sel!(saveFile:));
        unsafe {
            self.addSubview(&save_button);
        }
        *self.ivars().save_button.borrow_mut() = Some(save_button);

        // Create Save As button
        let save_as_frame = NSRect::new(
            NSPoint::new(save_as_x, (NOTIFICATION_BAR_HEIGHT - button_height) / 2.0),
            NSSize::new(button_width, button_height),
        );
        let save_as_button = unsafe { NSButton::initWithFrame(mtm.alloc(), save_as_frame) };
        Self::configure_button(mtm, &save_as_button, "Save As...", sel!(saveFileAs:));
        unsafe {
            self.addSubview(&save_as_button);
        }
        *self.ivars().save_as_button.borrow_mut() = Some(save_as_button);

        // Create Discard button
        let discard_frame = NSRect::new(
            NSPoint::new(discard_x, (NOTIFICATION_BAR_HEIGHT - button_height) / 2.0),
            NSSize::new(button_width, button_height),
        );
        let discard_button = unsafe { NSButton::initWithFrame(mtm.alloc(), discard_frame) };
        Self::configure_button(mtm, &discard_button, "Discard", sel!(discardFile:));
        unsafe {
            self.addSubview(&discard_button);
        }
        *self.ivars().discard_button.borrow_mut() = Some(discard_button);
    }

    /// Show notification for a received file
    pub fn show_file(&self, id: u64, name: Option<&str>, size: usize) {
        self.ivars().file_id.set(id);

        let display_name = name.unwrap_or("unnamed file");
        let size_str = format_size(size);
        let text = format!("Received file: {} ({})", display_name, size_str);

        if let Some(ref label) = *self.ivars().label.borrow() {
            label.setStringValue(&NSString::from_str(&text));
        }

        self.setHidden(false);

        log::debug!("Showing notification for file {} (id={})", display_name, id);
    }

    /// Hide the notification bar
    pub fn hide(&self) {
        self.setHidden(true);
        self.ivars().file_id.set(0);
    }

    /// Get the current file ID
    pub fn file_id(&self) -> u64 {
        self.ivars().file_id.get()
    }

    /// Update the bar width when window resizes
    pub fn update_width(&self, width: f64) {
        let mut frame = self.frame();
        frame.size.width = width;
        self.setFrame(frame);

        // Reposition buttons
        let padding = 8.0;
        let button_width = 70.0;
        let button_height = 22.0;
        let button_spacing = 8.0;

        let discard_x = width - padding - button_width;
        let save_as_x = discard_x - button_spacing - button_width;
        let save_x = save_as_x - button_spacing - button_width;
        let label_width = save_x - padding - padding;

        if let Some(ref label) = *self.ivars().label.borrow() {
            let mut frame = label.frame();
            frame.size.width = label_width;
            label.setFrame(frame);
        }

        if let Some(ref button) = *self.ivars().save_button.borrow() {
            let mut frame = button.frame();
            frame.origin.x = save_x;
            button.setFrame(frame);
        }

        if let Some(ref button) = *self.ivars().save_as_button.borrow() {
            let mut frame = button.frame();
            frame.origin.x = save_as_x;
            button.setFrame(frame);
        }

        if let Some(ref button) = *self.ivars().discard_button.borrow() {
            let mut frame = button.frame();
            frame.origin.x = discard_x;
            button.setFrame(frame);
        }
    }

    /// Get the frame rectangle
    fn frame(&self) -> NSRect {
        unsafe { msg_send![self, frame] }
    }

    /// Set the frame rectangle
    #[allow(non_snake_case)]
    fn setFrame(&self, frame: NSRect) {
        unsafe {
            let _: () = msg_send![self, setFrame: frame];
        }
    }

    /// Set hidden state
    #[allow(non_snake_case)]
    fn setHidden(&self, hidden: bool) {
        unsafe {
            let _: () = msg_send![self, setHidden: hidden];
        }
    }

    /// Check if hidden
    pub fn is_hidden(&self) -> bool {
        unsafe { msg_send![self, isHidden] }
    }

    /// Configure a button with title and action for dark background
    fn configure_button(
        mtm: MainThreadMarker,
        button: &NSButton,
        title: &str,
        action: objc2::runtime::Sel,
    ) {
        // Use momentary push button with recessed style (works better on dark backgrounds)
        button.setButtonType(NSButtonType::MomentaryPushIn);
        button.setBezelStyle(NSBezelStyle::Recessed);
        button.setBordered(true);

        unsafe {
            let title_str = NSString::from_str(title);

            // Create attributed string with white foreground color
            let white = NSColor::whiteColor();
            let fg_key = NSString::from_str("NSColor"); // NSForegroundColorAttributeName

            // Create dictionary with foreground color
            let keys: [&NSString; 1] = [&fg_key];
            let objects: [&objc2::runtime::AnyObject; 1] = [&*white];
            let attrs = NSDictionary::from_slices(&keys, &objects);

            let attr_title = NSAttributedString::initWithString_attributes(
                mtm.alloc(),
                &title_str,
                Some(&attrs),
            );

            button.setAttributedTitle(&attr_title);
            button.setTarget(None);
            button.setAction(Some(action));
        }
    }

    /// Set the action target for all buttons (call after adding to view hierarchy)
    /// Uses raw msg_send! to preserve the target's actual type information
    pub fn set_action_target<T: objc2::Message>(&self, target: &T) {
        unsafe {
            if let Some(ref button) = *self.ivars().save_button.borrow() {
                let _: () = msg_send![button, setTarget: target];
            }
            if let Some(ref button) = *self.ivars().save_as_button.borrow() {
                let _: () = msg_send![button, setTarget: target];
            }
            if let Some(ref button) = *self.ivars().discard_button.borrow() {
                let _: () = msg_send![button, setTarget: target];
            }
        }
    }

    /// Set wants layer
    #[allow(non_snake_case)]
    fn setWantsLayer(&self, wants: bool) {
        unsafe {
            let _: () = msg_send![self, setWantsLayer: wants];
        }
    }

    /// Get the layer
    fn layer(&self) -> Option<Retained<objc2::runtime::AnyObject>> {
        unsafe { msg_send![self, layer] }
    }
}
