//! Notification bar for file transfer UI
//!
//! Shows a dismissible notification when files are received via iTerm2 protocol.
//! Format: "Received file: Name.bin (1.2 MB)" [Save] [Save As...] [Discard]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cterm_ui::format_size;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, CssProvider, Label, Orientation};

/// Notification bar height in pixels
pub const NOTIFICATION_BAR_HEIGHT: i32 = 32;

/// Callback type for save actions
type SaveCallback = Rc<RefCell<Option<Box<dyn Fn(u64)>>>>;
/// Callback type for save-as actions
type SaveAsCallback = Rc<RefCell<Option<Box<dyn Fn(u64)>>>>;
/// Callback type for discard actions
type DiscardCallback = Rc<RefCell<Option<Box<dyn Fn(u64)>>>>;

/// Notification bar widget for file transfers
#[derive(Clone)]
pub struct NotificationBar {
    /// Main container widget
    container: GtkBox,
    /// Label showing file name and size
    label: Label,
    /// Save button (kept for potential future use)
    #[allow(dead_code)]
    save_button: Button,
    /// Save As button (kept for potential future use)
    #[allow(dead_code)]
    save_as_button: Button,
    /// Discard button (kept for potential future use)
    #[allow(dead_code)]
    discard_button: Button,
    /// Current file ID
    file_id: Rc<Cell<u64>>,
    /// Save callback
    on_save: SaveCallback,
    /// Save As callback
    on_save_as: SaveAsCallback,
    /// Discard callback
    on_discard: DiscardCallback,
}

impl NotificationBar {
    /// Create a new notification bar
    pub fn new() -> Self {
        // Create main container
        let container = GtkBox::new(Orientation::Horizontal, 8);
        container.set_height_request(NOTIFICATION_BAR_HEIGHT);
        container.add_css_class("notification-bar");

        // Apply dark semi-transparent styling
        let provider = CssProvider::new();
        provider.load_from_data(
            r#"
            .notification-bar {
                background-color: rgba(40, 40, 40, 0.95);
                padding: 4px 8px;
            }
            .notification-bar label {
                color: white;
            }
            .notification-bar button {
                min-height: 0;
                padding: 2px 12px;
            }
        "#,
        );
        container
            .style_context()
            .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);

        // Create label
        let label = Label::new(None);
        label.set_hexpand(true);
        label.set_halign(gtk4::Align::Start);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        container.append(&label);

        // Create Save button
        let save_button = Button::with_label("Save");
        container.append(&save_button);

        // Create Save As button
        let save_as_button = Button::with_label("Save As...");
        container.append(&save_as_button);

        // Create Discard button
        let discard_button = Button::with_label("Discard");
        container.append(&discard_button);

        // Initially hidden
        container.set_visible(false);

        let file_id = Rc::new(Cell::new(0u64));
        let on_save: SaveCallback = Rc::new(RefCell::new(None));
        let on_save_as: SaveAsCallback = Rc::new(RefCell::new(None));
        let on_discard: DiscardCallback = Rc::new(RefCell::new(None));

        // Connect button signals
        {
            let file_id = Rc::clone(&file_id);
            let on_save = Rc::clone(&on_save);
            save_button.connect_clicked(move |_| {
                let id = file_id.get();
                if let Some(ref cb) = *on_save.borrow() {
                    cb(id);
                }
            });
        }

        {
            let file_id = Rc::clone(&file_id);
            let on_save_as = Rc::clone(&on_save_as);
            save_as_button.connect_clicked(move |_| {
                let id = file_id.get();
                if let Some(ref cb) = *on_save_as.borrow() {
                    cb(id);
                }
            });
        }

        {
            let file_id = Rc::clone(&file_id);
            let on_discard = Rc::clone(&on_discard);
            discard_button.connect_clicked(move |_| {
                let id = file_id.get();
                if let Some(ref cb) = *on_discard.borrow() {
                    cb(id);
                }
            });
        }

        Self {
            container,
            label,
            save_button,
            save_as_button,
            discard_button,
            file_id,
            on_save,
            on_save_as,
            on_discard,
        }
    }

    /// Get the widget to add to the UI
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Show notification for a received file
    pub fn show_file(&self, id: u64, name: Option<&str>, size: usize) {
        self.file_id.set(id);

        let display_name = name.unwrap_or("unnamed file");
        let size_str = format_size(size);
        let text = format!("Received file: {} ({})", display_name, size_str);

        self.label.set_text(&text);
        self.container.set_visible(true);

        log::debug!("Showing notification for file {} (id={})", display_name, id);
    }

    /// Hide the notification bar
    pub fn hide(&self) {
        self.container.set_visible(false);
        self.file_id.set(0);
    }

    /// Check if the notification bar is visible
    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        self.container.is_visible()
    }

    /// Get the current file ID
    #[allow(dead_code)]
    pub fn file_id(&self) -> u64 {
        self.file_id.get()
    }

    /// Set callback for Save button
    pub fn set_on_save<F>(&self, callback: F)
    where
        F: Fn(u64) + 'static,
    {
        *self.on_save.borrow_mut() = Some(Box::new(callback));
    }

    /// Set callback for Save As button
    pub fn set_on_save_as<F>(&self, callback: F)
    where
        F: Fn(u64) + 'static,
    {
        *self.on_save_as.borrow_mut() = Some(Box::new(callback));
    }

    /// Set callback for Discard button
    pub fn set_on_discard<F>(&self, callback: F)
    where
        F: Fn(u64) + 'static,
    {
        *self.on_discard.borrow_mut() = Some(Box::new(callback));
    }
}

impl Default for NotificationBar {
    fn default() -> Self {
        Self::new()
    }
}
