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

use cterm_app::config::{save_sticky_tabs, DockerMode, DockerTabConfig, StickyTabConfig};

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
    // Docker fields
    docker_mode_popup: RefCell<Option<Retained<NSPopUpButton>>>,
    docker_container_field: RefCell<Option<Retained<NSTextField>>>,
    docker_image_field: RefCell<Option<Retained<NSTextField>>>,
    docker_shell_field: RefCell<Option<Retained<NSTextField>>>,
    docker_auto_remove_checkbox: RefCell<Option<Retained<NSButton>>>,
    docker_project_dir_field: RefCell<Option<Retained<NSTextField>>>,
    docker_status_label: RefCell<Option<Retained<NSTextField>>>,
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
        fn control_text_did_change(&self, notification: &NSNotification) {
            // Auto-save field changes to the selected template
            if let Some(index) = *self.ivars().selected_index.borrow() {
                self.save_fields_to_template(index);
                // Update popup button title if name changed
                self.update_popup_item_title(index);

                // Check if the changed field is the project directory field
                // and auto-detect devcontainer.json
                if let Some(changed_field) = notification.object() {
                    if let Some(project_dir_field) =
                        self.ivars().docker_project_dir_field.borrow().as_ref()
                    {
                        // Compare object pointers to see if it's the project dir field
                        let changed_ptr: *const AnyObject = &*changed_field;
                        let project_ptr: *const AnyObject =
                            (project_dir_field as &AnyObject) as *const AnyObject;
                        if changed_ptr == project_ptr {
                            self.auto_detect_devcontainer();
                        }
                    }
                }
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

        #[unsafe(method(dockerModeChanged:))]
        fn action_docker_mode_changed(&self, _sender: Option<&AnyObject>) {
            // Save and update UI visibility based on mode
            if let Some(index) = *self.ivars().selected_index.borrow() {
                self.save_fields_to_template(index);
                self.update_docker_fields_visibility();
            }
        }

        #[unsafe(method(addPreset:))]
        fn action_add_preset(&self, sender: Option<&AnyObject>) {
            if let Some(popup) = sender.and_then(|s| unsafe {
                let popup: *const NSPopUpButton = s as *const AnyObject as *const NSPopUpButton;
                Some(&*popup)
            }) {
                let index = popup.indexOfSelectedItem();
                self.add_preset_template(index as usize);
                // Reset popup to first item (the label)
                popup.selectItemAtIndex(0);
            }
        }
    }
);

impl TabTemplatesWindow {
    /// Create and show the tab templates window
    pub fn new(mtm: MainThreadMarker, templates: Vec<StickyTabConfig>) -> Retained<Self> {
        let content_rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(550.0, 620.0));

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
            // Docker fields
            docker_mode_popup: RefCell::new(None),
            docker_container_field: RefCell::new(None),
            docker_image_field: RefCell::new(None),
            docker_shell_field: RefCell::new(None),
            docker_auto_remove_checkbox: RefCell::new(None),
            docker_project_dir_field: RefCell::new(None),
            docker_status_label: RefCell::new(None),
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

        // Prevent double-free when window closes - Rust manages the lifetime
        unsafe { this.setReleasedWhenClosed(false) };

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

        // Presets dropdown
        let presets_popup = unsafe { NSPopUpButton::new(mtm) };
        presets_popup.removeAllItems();
        presets_popup.addItemWithTitle(&NSString::from_str("Add Preset..."));
        presets_popup.addItemWithTitle(&NSString::from_str("Claude Code"));
        presets_popup.addItemWithTitle(&NSString::from_str("Claude Code (Container)"));
        presets_popup.addItemWithTitle(&NSString::from_str("Ubuntu Container"));
        presets_popup.addItemWithTitle(&NSString::from_str("Alpine Container"));
        presets_popup.addItemWithTitle(&NSString::from_str("Node.js Container"));
        presets_popup.addItemWithTitle(&NSString::from_str("Python Container"));
        unsafe { presets_popup.setTarget(Some(self)) };
        unsafe { presets_popup.setAction(Some(sel!(addPreset:))) };

        selector_row.addView_inGravity(&selector_label, NSStackViewGravity::Leading);
        selector_row.addView_inGravity(&popup, NSStackViewGravity::Leading);
        selector_row.addView_inGravity(&add_btn, NSStackViewGravity::Leading);
        selector_row.addView_inGravity(&remove_btn, NSStackViewGravity::Leading);
        selector_row.addView_inGravity(&presets_popup, NSStackViewGravity::Leading);

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

        // Docker section separator
        let docker_separator = unsafe { objc2_app_kit::NSBox::new(mtm) };
        unsafe { docker_separator.setBoxType(objc2_app_kit::NSBoxType::Separator) };
        main_stack.addView_inGravity(&docker_separator, NSStackViewGravity::Top);

        // Docker section label
        let docker_label = unsafe {
            NSTextField::labelWithString(&NSString::from_str("Docker Configuration"), mtm)
        };
        unsafe {
            docker_label.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(
                objc2_app_kit::NSFont::systemFontSize(),
            )))
        };
        main_stack.addView_inGravity(&docker_label, NSStackViewGravity::Top);

        // Docker mode dropdown
        let docker_mode_row = unsafe { NSStackView::new(mtm) };
        docker_mode_row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        docker_mode_row.setSpacing(10.0);

        let docker_mode_label =
            unsafe { NSTextField::labelWithString(&NSString::from_str("Docker Mode:"), mtm) };
        unsafe { docker_mode_label.setFrameSize(NSSize::new(100.0, 22.0)) };

        let docker_mode_popup = unsafe { NSPopUpButton::new(mtm) };
        docker_mode_popup.removeAllItems();
        docker_mode_popup.addItemWithTitle(&NSString::from_str("None (Regular Tab)"));
        docker_mode_popup.addItemWithTitle(&NSString::from_str("Exec (Connect to Container)"));
        docker_mode_popup.addItemWithTitle(&NSString::from_str("Run (Start Container)"));
        docker_mode_popup.addItemWithTitle(&NSString::from_str("DevContainer (With Mounts)"));
        unsafe { docker_mode_popup.setTarget(Some(self)) };
        unsafe { docker_mode_popup.setAction(Some(sel!(dockerModeChanged:))) };

        docker_mode_row.addView_inGravity(&docker_mode_label, NSStackViewGravity::Leading);
        docker_mode_row.addView_inGravity(&docker_mode_popup, NSStackViewGravity::Leading);
        main_stack.addView_inGravity(&docker_mode_row, NSStackViewGravity::Top);
        *self.ivars().docker_mode_popup.borrow_mut() = Some(docker_mode_popup);

        // Container field (for Exec mode)
        let container_row = self.create_field_row(mtm, "Container:", 200.0);
        main_stack.addView_inGravity(&container_row.0, NSStackViewGravity::Top);
        *self.ivars().docker_container_field.borrow_mut() = Some(container_row.1);

        // Image field (for Run/DevContainer modes)
        let image_row = self.create_field_row(mtm, "Image:", 200.0);
        main_stack.addView_inGravity(&image_row.0, NSStackViewGravity::Top);
        *self.ivars().docker_image_field.borrow_mut() = Some(image_row.1);

        // Shell field
        let shell_row = self.create_field_row(mtm, "Shell:", 150.0);
        main_stack.addView_inGravity(&shell_row.0, NSStackViewGravity::Top);
        *self.ivars().docker_shell_field.borrow_mut() = Some(shell_row.1);

        // Docker checkboxes
        let docker_auto_remove_cb = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str("Auto-remove container on exit"),
                Some(self),
                Some(sel!(checkboxChanged:)),
                mtm,
            )
        };
        main_stack.addView_inGravity(&docker_auto_remove_cb, NSStackViewGravity::Top);
        *self.ivars().docker_auto_remove_checkbox.borrow_mut() = Some(docker_auto_remove_cb);

        // Project directory field (for DevContainer mode - reads .devcontainer/devcontainer.json)
        let project_dir_row = self.create_field_row(mtm, "Project Dir:", 250.0);
        main_stack.addView_inGravity(&project_dir_row.0, NSStackViewGravity::Top);
        *self.ivars().docker_project_dir_field.borrow_mut() = Some(project_dir_row.1);

        // Status label for devcontainer detection
        let status_label = unsafe { NSTextField::labelWithString(&NSString::from_str(""), mtm) };
        unsafe {
            status_label.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
        }
        main_stack.addView_inGravity(&status_label, NSStackViewGravity::Top);
        *self.ivars().docker_status_label.borrow_mut() = Some(status_label);

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

            // Docker fields
            if let Some(popup) = self.ivars().docker_mode_popup.borrow().as_ref() {
                let mode_index = match &template.docker {
                    None => 0,
                    Some(docker) => match docker.mode {
                        DockerMode::Exec => 1,
                        DockerMode::Run => 2,
                        DockerMode::DevContainer => 3,
                    },
                };
                popup.selectItemAtIndex(mode_index);
            }

            if let Some(field) = self.ivars().docker_container_field.borrow().as_ref() {
                let container = template
                    .docker
                    .as_ref()
                    .and_then(|d| d.container.as_deref())
                    .unwrap_or("");
                field.setStringValue(&NSString::from_str(container));
            }

            if let Some(field) = self.ivars().docker_image_field.borrow().as_ref() {
                let image = template
                    .docker
                    .as_ref()
                    .and_then(|d| d.image.as_deref())
                    .unwrap_or("");
                field.setStringValue(&NSString::from_str(image));
            }

            if let Some(field) = self.ivars().docker_shell_field.borrow().as_ref() {
                let shell = template
                    .docker
                    .as_ref()
                    .and_then(|d| d.shell.as_deref())
                    .unwrap_or("");
                field.setStringValue(&NSString::from_str(shell));
            }

            if let Some(cb) = self.ivars().docker_auto_remove_checkbox.borrow().as_ref() {
                let auto_remove = template
                    .docker
                    .as_ref()
                    .map(|d| d.auto_remove)
                    .unwrap_or(true);
                cb.setState(if auto_remove { 1 } else { 0 });
            }

            if let Some(field) = self.ivars().docker_project_dir_field.borrow().as_ref() {
                let project_dir = template
                    .docker
                    .as_ref()
                    .and_then(|d| d.project_dir.as_ref())
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                field.setStringValue(&NSString::from_str(&project_dir));
            }

            self.update_docker_fields_visibility();
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

        // Clear Docker fields
        if let Some(popup) = self.ivars().docker_mode_popup.borrow().as_ref() {
            popup.selectItemAtIndex(0);
        }
        if let Some(field) = self.ivars().docker_container_field.borrow().as_ref() {
            field.setStringValue(&empty);
        }
        if let Some(field) = self.ivars().docker_image_field.borrow().as_ref() {
            field.setStringValue(&empty);
        }
        if let Some(field) = self.ivars().docker_shell_field.borrow().as_ref() {
            field.setStringValue(&empty);
        }
        if let Some(cb) = self.ivars().docker_auto_remove_checkbox.borrow().as_ref() {
            cb.setState(1); // Default to true
        }
        if let Some(field) = self.ivars().docker_project_dir_field.borrow().as_ref() {
            field.setStringValue(&empty);
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

            // Save Docker fields
            if let Some(popup) = self.ivars().docker_mode_popup.borrow().as_ref() {
                let mode_index = popup.indexOfSelectedItem();
                if mode_index == 0 {
                    // None - remove Docker config
                    template.docker = None;
                } else {
                    // Get or create Docker config
                    let docker = template.docker.get_or_insert_with(DockerTabConfig::default);

                    docker.mode = match mode_index {
                        1 => DockerMode::Exec,
                        2 => DockerMode::Run,
                        3 => DockerMode::DevContainer,
                        _ => DockerMode::Exec,
                    };

                    if let Some(field) = self.ivars().docker_container_field.borrow().as_ref() {
                        let container = field.stringValue().to_string();
                        docker.container = if container.is_empty() {
                            None
                        } else {
                            Some(container)
                        };
                    }

                    if let Some(field) = self.ivars().docker_image_field.borrow().as_ref() {
                        let image = field.stringValue().to_string();
                        docker.image = if image.is_empty() { None } else { Some(image) };
                    }

                    if let Some(field) = self.ivars().docker_shell_field.borrow().as_ref() {
                        let shell = field.stringValue().to_string();
                        docker.shell = if shell.is_empty() { None } else { Some(shell) };
                    }

                    if let Some(cb) = self.ivars().docker_auto_remove_checkbox.borrow().as_ref() {
                        docker.auto_remove = cb.state() != 0;
                    }

                    if let Some(field) = self.ivars().docker_project_dir_field.borrow().as_ref() {
                        let project_dir = field.stringValue().to_string();
                        docker.project_dir = if project_dir.is_empty() {
                            None
                        } else {
                            Some(PathBuf::from(project_dir))
                        };
                    }
                }
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

    fn update_docker_fields_visibility(&self) {
        // Get the current Docker mode
        let mode_index = self
            .ivars()
            .docker_mode_popup
            .borrow()
            .as_ref()
            .map(|p| p.indexOfSelectedItem())
            .unwrap_or(0);

        let is_docker = mode_index > 0;
        let is_exec = mode_index == 1;
        let is_run_or_devcontainer = mode_index >= 2;
        let is_devcontainer = mode_index == 3;

        // Show/hide container field (only for Exec mode)
        if let Some(field) = self.ivars().docker_container_field.borrow().as_ref() {
            field.setEnabled(is_exec);
            if let Some(superview) = unsafe { field.superview() } {
                superview.setHidden(!is_exec);
            }
        }

        // Show/hide image field (for Run and DevContainer modes)
        if let Some(field) = self.ivars().docker_image_field.borrow().as_ref() {
            field.setEnabled(is_run_or_devcontainer);
            if let Some(superview) = unsafe { field.superview() } {
                superview.setHidden(!is_run_or_devcontainer);
            }
        }

        // Show/hide shell field (for all Docker modes)
        if let Some(field) = self.ivars().docker_shell_field.borrow().as_ref() {
            field.setEnabled(is_docker);
            if let Some(superview) = unsafe { field.superview() } {
                superview.setHidden(!is_docker);
            }
        }

        // Show/hide auto-remove checkbox (for Run and DevContainer)
        if let Some(cb) = self.ivars().docker_auto_remove_checkbox.borrow().as_ref() {
            cb.setEnabled(is_run_or_devcontainer);
            cb.setHidden(!is_run_or_devcontainer);
        }

        // Show/hide project directory field (only for DevContainer - reads .devcontainer/devcontainer.json)
        if let Some(field) = self.ivars().docker_project_dir_field.borrow().as_ref() {
            field.setEnabled(is_devcontainer);
            if let Some(superview) = unsafe { field.superview() } {
                superview.setHidden(!is_devcontainer);
            }
        }
    }

    fn add_preset_template(&self, preset_index: usize) {
        let new_template = match preset_index {
            1 => {
                // Claude Code (native)
                StickyTabConfig::claude()
            }
            2 => {
                // Claude Code (Container)
                StickyTabConfig::claude_devcontainer(None)
            }
            3 => {
                // Ubuntu Container
                StickyTabConfig {
                    name: "Ubuntu".into(),
                    color: Some("#E95420".into()), // Ubuntu orange
                    keep_open: true,
                    docker: Some(DockerTabConfig {
                        mode: DockerMode::Run,
                        image: Some("ubuntu:latest".into()),
                        shell: Some("/bin/bash".into()),
                        auto_remove: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            }
            4 => {
                // Alpine Container
                StickyTabConfig {
                    name: "Alpine".into(),
                    color: Some("#0D597F".into()), // Alpine blue
                    keep_open: true,
                    docker: Some(DockerTabConfig {
                        mode: DockerMode::Run,
                        image: Some("alpine:latest".into()),
                        shell: Some("/bin/sh".into()),
                        auto_remove: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            }
            5 => {
                // Node.js Container
                StickyTabConfig {
                    name: "Node.js".into(),
                    color: Some("#339933".into()), // Node green
                    keep_open: true,
                    docker: Some(DockerTabConfig {
                        mode: DockerMode::Run,
                        image: Some("node:20".into()),
                        shell: Some("/bin/bash".into()),
                        auto_remove: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            }
            6 => {
                // Python Container
                StickyTabConfig {
                    name: "Python".into(),
                    color: Some("#3776AB".into()), // Python blue
                    keep_open: true,
                    docker: Some(DockerTabConfig {
                        mode: DockerMode::Run,
                        image: Some("python:3.12".into()),
                        shell: Some("/bin/bash".into()),
                        auto_remove: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            }
            _ => return, // Index 0 is "Add Preset..." label, do nothing
        };

        let new_index = {
            let mut templates = self.ivars().templates.borrow_mut();
            templates.push(new_template.clone());
            templates.len() - 1
        };

        // Add to popup button
        if let Some(popup) = self.ivars().template_selector.borrow().as_ref() {
            popup.addItemWithTitle(&NSString::from_str(&new_template.name));
            popup.selectItemAtIndex(new_index as isize);
        }

        *self.ivars().selected_index.borrow_mut() = Some(new_index);
        self.load_template_into_fields(new_index);
    }

    /// Auto-detect devcontainer.json when project directory changes
    fn auto_detect_devcontainer(&self) {
        // Get the project directory from the field
        let project_dir = self
            .ivars()
            .docker_project_dir_field
            .borrow()
            .as_ref()
            .map(|f| f.stringValue().to_string())
            .unwrap_or_default();

        if project_dir.is_empty() {
            // Clear status and don't change mode
            if let Some(label) = self.ivars().docker_status_label.borrow().as_ref() {
                label.setStringValue(&NSString::from_str(""));
            }
            return;
        }

        let path = PathBuf::from(&project_dir);

        // Check for .devcontainer/devcontainer.json
        let devcontainer_path = path.join(".devcontainer/devcontainer.json");
        let alt_devcontainer_path = path.join(".devcontainer.json");

        let found = devcontainer_path.exists() || alt_devcontainer_path.exists();

        if found {
            // Update status label
            if let Some(label) = self.ivars().docker_status_label.borrow().as_ref() {
                label.setStringValue(&NSString::from_str("✓ devcontainer.json detected"));
                unsafe {
                    label.setTextColor(Some(&objc2_app_kit::NSColor::systemGreenColor()));
                }
            }

            // Auto-switch to DevContainer mode if currently set to None
            if let Some(popup) = self.ivars().docker_mode_popup.borrow().as_ref() {
                let current_mode = popup.indexOfSelectedItem();
                if current_mode == 0 {
                    // Currently "None", switch to DevContainer
                    popup.selectItemAtIndex(3); // DevContainer is index 3
                    self.update_docker_fields_visibility();

                    // Save the change
                    if let Some(index) = *self.ivars().selected_index.borrow() {
                        self.save_fields_to_template(index);
                    }
                }
            }
        } else {
            // Update status label
            if let Some(label) = self.ivars().docker_status_label.borrow().as_ref() {
                if path.exists() {
                    label.setStringValue(&NSString::from_str("No devcontainer.json found"));
                    unsafe {
                        label.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
                    }
                } else {
                    label.setStringValue(&NSString::from_str("Directory does not exist"));
                    unsafe {
                        label.setTextColor(Some(&objc2_app_kit::NSColor::systemOrangeColor()));
                    }
                }
            }
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
