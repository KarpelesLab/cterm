//! Tab Templates UI for macOS
//!
//! Provides a window for managing tab templates (sticky tabs).

use std::cell::RefCell;
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSButton, NSControlTextEditingDelegate, NSLayoutAttribute, NSPopUpButton, NSStackView,
    NSStackViewGravity, NSTextField, NSTextFieldDelegate, NSUserInterfaceLayoutOrientation,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use cterm_app::config::{save_sticky_tabs, StickyTabConfig};

/// State for the tab templates window
pub struct TabTemplatesWindowIvars {
    templates: RefCell<Vec<StickyTabConfig>>,
    selected_index: RefCell<Option<usize>>,
    template_selector: RefCell<Option<Retained<NSPopUpButton>>>,
    name_field: RefCell<Option<Retained<NSTextField>>>,
    command_field: RefCell<Option<Retained<NSTextField>>>,
    args_field: RefCell<Option<Retained<NSTextField>>>,
    path_field: RefCell<Option<Retained<NSTextField>>>,
    color_field: RefCell<Option<Retained<NSTextField>>>,
    theme_field: RefCell<Option<Retained<NSTextField>>>,
    unique_checkbox: RefCell<Option<Retained<NSButton>>>,
    auto_start_checkbox: RefCell<Option<Retained<NSButton>>>,
    keep_open_checkbox: RefCell<Option<Retained<NSButton>>>,
}

define_class!(
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "TabTemplatesWindow"]
    #[ivars = TabTemplatesWindowIvars]
    pub struct TabTemplatesWindow;

    unsafe impl NSObjectProtocol for TabTemplatesWindow {}

    unsafe impl NSTextFieldDelegate for TabTemplatesWindow {}

    unsafe impl NSControlTextEditingDelegate for TabTemplatesWindow {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _notification: &NSNotification) {
            // Auto-save field changes to the selected template
            if let Some(index) = *self.ivars().selected_index.borrow() {
                self.save_fields_to_template(index);
                // Update popup button title if name changed
                self.update_popup_item_title(index);
            }
        }
    }

    // Button actions
    impl TabTemplatesWindow {
        #[unsafe(method(templateSelected:))]
        fn action_template_selected(&self, _sender: Option<&AnyObject>) {
            if let Some(popup) = self.ivars().template_selector.borrow().as_ref() {
                let index = popup.indexOfSelectedItem();
                if index >= 0 {
                    *self.ivars().selected_index.borrow_mut() = Some(index as usize);
                    self.load_template_into_fields(index as usize);
                }
            }
        }

        #[unsafe(method(addTemplate:))]
        fn action_add_template(&self, _sender: Option<&AnyObject>) {
            self.add_new_template();
        }

        #[unsafe(method(removeTemplate:))]
        fn action_remove_template(&self, _sender: Option<&AnyObject>) {
            self.remove_selected_template();
        }

        #[unsafe(method(saveAndClose:))]
        fn action_save_and_close(&self, _sender: Option<&AnyObject>) {
            self.save_templates();
            self.close();
        }

        #[unsafe(method(cancelClose:))]
        fn action_cancel(&self, _sender: Option<&AnyObject>) {
            self.close();
        }

        #[unsafe(method(checkboxChanged:))]
        fn action_checkbox_changed(&self, _sender: Option<&AnyObject>) {
            // Save checkbox changes to the selected template
            if let Some(index) = *self.ivars().selected_index.borrow() {
                self.save_fields_to_template(index);
            }
        }
    }
);

impl TabTemplatesWindow {
    /// Create and show the tab templates window
    pub fn new(mtm: MainThreadMarker, templates: Vec<StickyTabConfig>) -> Retained<Self> {
        let content_rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(500.0, 450.0));

        let style_mask =
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;

        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(TabTemplatesWindowIvars {
            templates: RefCell::new(templates),
            selected_index: RefCell::new(None),
            template_selector: RefCell::new(None),
            name_field: RefCell::new(None),
            command_field: RefCell::new(None),
            args_field: RefCell::new(None),
            path_field: RefCell::new(None),
            color_field: RefCell::new(None),
            theme_field: RefCell::new(None),
            unique_checkbox: RefCell::new(None),
            auto_start_checkbox: RefCell::new(None),
            keep_open_checkbox: RefCell::new(None),
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

        this.setTitle(&NSString::from_str("Tab Templates"));
        this.setMinSize(NSSize::new(400.0, 350.0));

        // Build the UI
        this.build_ui(mtm);

        // Select first template if available
        if !this.ivars().templates.borrow().is_empty() {
            *this.ivars().selected_index.borrow_mut() = Some(0);
            this.load_template_into_fields(0);
        }

        this
    }

    fn build_ui(&self, mtm: MainThreadMarker) {
        // Create main vertical stack
        let main_stack = unsafe { NSStackView::new(mtm) };
        main_stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        main_stack.setSpacing(15.0);
        main_stack.setAlignment(NSLayoutAttribute::Leading);
        unsafe {
            main_stack.setEdgeInsets(objc2_foundation::NSEdgeInsets {
                top: 20.0,
                left: 20.0,
                bottom: 20.0,
                right: 20.0,
            });
        }

        // Template selector row
        let selector_row = unsafe { NSStackView::new(mtm) };
        selector_row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        selector_row.setSpacing(10.0);

        let selector_label =
            unsafe { NSTextField::labelWithString(&NSString::from_str("Template:"), mtm) };

        let popup = unsafe { NSPopUpButton::new(mtm) };
        popup.removeAllItems();

        // Populate popup with templates
        for template in self.ivars().templates.borrow().iter() {
            popup.addItemWithTitle(&NSString::from_str(&template.name));
        }

        unsafe { popup.setTarget(Some(self)) };
        unsafe { popup.setAction(Some(sel!(templateSelected:))) };

        let add_btn = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("+"),
                Some(self),
                Some(sel!(addTemplate:)),
                mtm,
            )
        };
        let remove_btn = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("-"),
                Some(self),
                Some(sel!(removeTemplate:)),
                mtm,
            )
        };

        selector_row.addView_inGravity(&selector_label, NSStackViewGravity::Leading);
        selector_row.addView_inGravity(&popup, NSStackViewGravity::Leading);
        selector_row.addView_inGravity(&add_btn, NSStackViewGravity::Leading);
        selector_row.addView_inGravity(&remove_btn, NSStackViewGravity::Leading);

        *self.ivars().template_selector.borrow_mut() = Some(popup);

        main_stack.addView_inGravity(&selector_row, NSStackViewGravity::Top);

        // Separator
        let separator = unsafe { objc2_app_kit::NSBox::new(mtm) };
        unsafe { separator.setBoxType(objc2_app_kit::NSBoxType::Separator) };
        main_stack.addView_inGravity(&separator, NSStackViewGravity::Top);

        // Name field
        let name_row = self.create_field_row(mtm, "Name:", 250.0);
        main_stack.addView_inGravity(&name_row.0, NSStackViewGravity::Top);
        *self.ivars().name_field.borrow_mut() = Some(name_row.1);

        // Command field
        let command_row = self.create_field_row(mtm, "Command:", 250.0);
        main_stack.addView_inGravity(&command_row.0, NSStackViewGravity::Top);
        *self.ivars().command_field.borrow_mut() = Some(command_row.1);

        // Args field
        let args_row = self.create_field_row(mtm, "Arguments:", 250.0);
        main_stack.addView_inGravity(&args_row.0, NSStackViewGravity::Top);
        *self.ivars().args_field.borrow_mut() = Some(args_row.1);

        // Path field
        let path_row = self.create_field_row(mtm, "Working Dir:", 250.0);
        main_stack.addView_inGravity(&path_row.0, NSStackViewGravity::Top);
        *self.ivars().path_field.borrow_mut() = Some(path_row.1);

        // Color field
        let color_row = self.create_field_row(mtm, "Tab Color:", 120.0);
        main_stack.addView_inGravity(&color_row.0, NSStackViewGravity::Top);
        *self.ivars().color_field.borrow_mut() = Some(color_row.1);

        // Theme field
        let theme_row = self.create_field_row(mtm, "Theme:", 150.0);
        main_stack.addView_inGravity(&theme_row.0, NSStackViewGravity::Top);
        *self.ivars().theme_field.borrow_mut() = Some(theme_row.1);

        // Checkboxes
        let unique_cb = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str("Unique (only one instance allowed)"),
                Some(self),
                Some(sel!(checkboxChanged:)),
                mtm,
            )
        };
        main_stack.addView_inGravity(&unique_cb, NSStackViewGravity::Top);
        *self.ivars().unique_checkbox.borrow_mut() = Some(unique_cb);

        let auto_start_cb = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str("Auto-start on launch"),
                Some(self),
                Some(sel!(checkboxChanged:)),
                mtm,
            )
        };
        main_stack.addView_inGravity(&auto_start_cb, NSStackViewGravity::Top);
        *self.ivars().auto_start_checkbox.borrow_mut() = Some(auto_start_cb);

        let keep_open_cb = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str("Keep tab open after exit"),
                Some(self),
                Some(sel!(checkboxChanged:)),
                mtm,
            )
        };
        main_stack.addView_inGravity(&keep_open_cb, NSStackViewGravity::Top);
        *self.ivars().keep_open_checkbox.borrow_mut() = Some(keep_open_cb);

        // Bottom buttons
        let bottom_stack = unsafe { NSStackView::new(mtm) };
        bottom_stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        bottom_stack.setSpacing(10.0);

        let cancel_btn = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Cancel"),
                Some(self),
                Some(sel!(cancelClose:)),
                mtm,
            )
        };
        let save_btn = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Save"),
                Some(self),
                Some(sel!(saveAndClose:)),
                mtm,
            )
        };
        unsafe { save_btn.setBezelStyle(objc2_app_kit::NSBezelStyle::Rounded) };
        unsafe { save_btn.setKeyEquivalent(&NSString::from_str("\r")) }; // Enter key

        bottom_stack.addView_inGravity(&cancel_btn, NSStackViewGravity::Trailing);
        bottom_stack.addView_inGravity(&save_btn, NSStackViewGravity::Trailing);

        main_stack.addView_inGravity(&bottom_stack, NSStackViewGravity::Bottom);

        self.setContentView(Some(&main_stack));
    }

    fn create_field_row(
        &self,
        mtm: MainThreadMarker,
        label: &str,
        field_width: f64,
    ) -> (Retained<NSStackView>, Retained<NSTextField>) {
        let stack = unsafe { NSStackView::new(mtm) };
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        stack.setSpacing(10.0);

        let label_view = unsafe { NSTextField::labelWithString(&NSString::from_str(label), mtm) };
        unsafe { label_view.setFrameSize(NSSize::new(100.0, 22.0)) };

        let field = unsafe { NSTextField::new(mtm) };
        unsafe { field.setFrameSize(NSSize::new(field_width, 22.0)) };
        field.setEditable(true);
        field.setBezeled(true);

        // Set self as delegate for text change notifications
        unsafe {
            use objc2::runtime::ProtocolObject;
            let delegate: &ProtocolObject<dyn NSTextFieldDelegate> = ProtocolObject::from_ref(self);
            field.setDelegate(Some(delegate));
        }

        stack.addView_inGravity(&label_view, NSStackViewGravity::Leading);
        stack.addView_inGravity(&field, NSStackViewGravity::Leading);

        (stack, field)
    }

    fn load_template_into_fields(&self, index: usize) {
        let templates = self.ivars().templates.borrow();
        if let Some(template) = templates.get(index) {
            if let Some(field) = self.ivars().name_field.borrow().as_ref() {
                field.setStringValue(&NSString::from_str(&template.name));
            }
            if let Some(field) = self.ivars().command_field.borrow().as_ref() {
                field.setStringValue(&NSString::from_str(
                    template.command.as_deref().unwrap_or(""),
                ));
            }
            if let Some(field) = self.ivars().args_field.borrow().as_ref() {
                field.setStringValue(&NSString::from_str(&template.args.join(" ")));
            }
            if let Some(field) = self.ivars().path_field.borrow().as_ref() {
                let path_str = template
                    .working_directory
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                field.setStringValue(&NSString::from_str(&path_str));
            }
            if let Some(field) = self.ivars().color_field.borrow().as_ref() {
                field.setStringValue(&NSString::from_str(template.color.as_deref().unwrap_or("")));
            }
            if let Some(field) = self.ivars().theme_field.borrow().as_ref() {
                field.setStringValue(&NSString::from_str(template.theme.as_deref().unwrap_or("")));
            }
            if let Some(cb) = self.ivars().unique_checkbox.borrow().as_ref() {
                cb.setState(if template.unique { 1 } else { 0 });
            }
            if let Some(cb) = self.ivars().auto_start_checkbox.borrow().as_ref() {
                cb.setState(if template.auto_start { 1 } else { 0 });
            }
            if let Some(cb) = self.ivars().keep_open_checkbox.borrow().as_ref() {
                cb.setState(if template.keep_open { 1 } else { 0 });
            }
        }
    }

    fn clear_fields(&self) {
        let empty = NSString::from_str("");
        if let Some(field) = self.ivars().name_field.borrow().as_ref() {
            field.setStringValue(&empty);
        }
        if let Some(field) = self.ivars().command_field.borrow().as_ref() {
            field.setStringValue(&empty);
        }
        if let Some(field) = self.ivars().args_field.borrow().as_ref() {
            field.setStringValue(&empty);
        }
        if let Some(field) = self.ivars().path_field.borrow().as_ref() {
            field.setStringValue(&empty);
        }
        if let Some(field) = self.ivars().color_field.borrow().as_ref() {
            field.setStringValue(&empty);
        }
        if let Some(field) = self.ivars().theme_field.borrow().as_ref() {
            field.setStringValue(&empty);
        }
        if let Some(cb) = self.ivars().unique_checkbox.borrow().as_ref() {
            cb.setState(0);
        }
        if let Some(cb) = self.ivars().auto_start_checkbox.borrow().as_ref() {
            cb.setState(0);
        }
        if let Some(cb) = self.ivars().keep_open_checkbox.borrow().as_ref() {
            cb.setState(0);
        }
    }

    fn save_fields_to_template(&self, index: usize) {
        let mut templates = self.ivars().templates.borrow_mut();
        if let Some(template) = templates.get_mut(index) {
            if let Some(field) = self.ivars().name_field.borrow().as_ref() {
                template.name = field.stringValue().to_string();
            }
            if let Some(field) = self.ivars().command_field.borrow().as_ref() {
                let cmd = field.stringValue().to_string();
                template.command = if cmd.is_empty() { None } else { Some(cmd) };
            }
            if let Some(field) = self.ivars().args_field.borrow().as_ref() {
                let args_str = field.stringValue().to_string();
                template.args = if args_str.is_empty() {
                    Vec::new()
                } else {
                    args_str.split_whitespace().map(|s| s.to_string()).collect()
                };
            }
            if let Some(field) = self.ivars().path_field.borrow().as_ref() {
                let path_str = field.stringValue().to_string();
                template.working_directory = if path_str.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(path_str))
                };
            }
            if let Some(field) = self.ivars().color_field.borrow().as_ref() {
                let color = field.stringValue().to_string();
                template.color = if color.is_empty() { None } else { Some(color) };
            }
            if let Some(field) = self.ivars().theme_field.borrow().as_ref() {
                let theme = field.stringValue().to_string();
                template.theme = if theme.is_empty() { None } else { Some(theme) };
            }
            if let Some(cb) = self.ivars().unique_checkbox.borrow().as_ref() {
                template.unique = cb.state() != 0;
            }
            if let Some(cb) = self.ivars().auto_start_checkbox.borrow().as_ref() {
                template.auto_start = cb.state() != 0;
            }
            if let Some(cb) = self.ivars().keep_open_checkbox.borrow().as_ref() {
                template.keep_open = cb.state() != 0;
            }
        }
    }

    fn update_popup_item_title(&self, index: usize) {
        let templates = self.ivars().templates.borrow();
        if let Some(template) = templates.get(index) {
            if let Some(popup) = self.ivars().template_selector.borrow().as_ref() {
                if let Some(item) = popup.itemAtIndex(index as isize) {
                    item.setTitle(&NSString::from_str(&template.name));
                }
            }
        }
    }

    fn add_new_template(&self) {
        let new_template = StickyTabConfig {
            name: "New Template".into(),
            ..Default::default()
        };

        let new_index = {
            let mut templates = self.ivars().templates.borrow_mut();
            templates.push(new_template);
            templates.len() - 1
        };

        // Add to popup button
        if let Some(popup) = self.ivars().template_selector.borrow().as_ref() {
            popup.addItemWithTitle(&NSString::from_str("New Template"));
            popup.selectItemAtIndex(new_index as isize);
        }

        *self.ivars().selected_index.borrow_mut() = Some(new_index);
        self.load_template_into_fields(new_index);
    }

    fn remove_selected_template(&self) {
        let selected = *self.ivars().selected_index.borrow();
        if let Some(index) = selected {
            let templates_len = {
                let mut templates = self.ivars().templates.borrow_mut();
                if index < templates.len() && templates.len() > 1 {
                    templates.remove(index);
                }
                templates.len()
            };

            // Rebuild popup button
            if let Some(popup) = self.ivars().template_selector.borrow().as_ref() {
                popup.removeAllItems();
                for template in self.ivars().templates.borrow().iter() {
                    popup.addItemWithTitle(&NSString::from_str(&template.name));
                }

                // Select previous or first item
                let new_index = if index > 0 { index - 1 } else { 0 };
                if templates_len > 0 {
                    popup.selectItemAtIndex(new_index as isize);
                    *self.ivars().selected_index.borrow_mut() = Some(new_index);
                    self.load_template_into_fields(new_index);
                } else {
                    *self.ivars().selected_index.borrow_mut() = None;
                    self.clear_fields();
                }
            }
        }
    }

    fn save_templates(&self) {
        let templates = self.ivars().templates.borrow();
        if let Err(e) = save_sticky_tabs(&templates) {
            log::error!("Failed to save tab templates: {}", e);
        } else {
            log::info!(
                "Tab templates saved successfully ({} templates)",
                templates.len()
            );
        }
    }
}

/// Show the tab templates window
pub fn show_tab_templates(
    mtm: MainThreadMarker,
    templates: Vec<StickyTabConfig>,
) -> Retained<TabTemplatesWindow> {
    let window = TabTemplatesWindow::new(mtm, templates);
    window.makeKeyAndOrderFront(None);
    window
}
