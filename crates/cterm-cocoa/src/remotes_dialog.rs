//! Manage Remotes dialog for macOS
//!
//! Simple dialog for adding/removing remote hosts that templates can target.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSButton, NSControlTextEditingDelegate, NSLayoutAttribute, NSStackView, NSStackViewGravity,
    NSTextField, NSTextFieldDelegate, NSUserInterfaceLayoutOrientation, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use cterm_app::config::{save_config, Config, ConnectionMethod, RemoteConfig};
use std::cell::RefCell;

pub struct RemotesDialogIvars {
    config: RefCell<Config>,
    list_popup: RefCell<Option<Retained<objc2_app_kit::NSPopUpButton>>>,
    name_field: RefCell<Option<Retained<NSTextField>>>,
    host_field: RefCell<Option<Retained<NSTextField>>>,
    method_popup: RefCell<Option<Retained<objc2_app_kit::NSPopUpButton>>>,
}

define_class!(
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "RemotesDialog"]
    #[ivars = RemotesDialogIvars]
    pub struct RemotesDialog;

    unsafe impl NSObjectProtocol for RemotesDialog {}

    unsafe impl NSTextFieldDelegate for RemotesDialog {}

    unsafe impl NSControlTextEditingDelegate for RemotesDialog {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _notification: &NSNotification) {
            self.save_current_to_config();
        }
    }

    // Action handlers
    impl RemotesDialog {
        #[unsafe(method(addRemote:))]
        fn action_add(&self, _sender: Option<&AnyObject>) {
            let mut config = self.ivars().config.borrow_mut();
            let name = format!("remote-{}", config.remotes.len() + 1);
            config.remotes.push(RemoteConfig {
                name,
                host: String::new(),
                method: Default::default(),
                ssh_compression: true,
            });
            let new_idx = config.remotes.len() - 1;
            drop(config);
            self.refresh_list();
            if let Some(popup) = self.ivars().list_popup.borrow().as_ref() {
                popup.selectItemAtIndex(new_idx as isize);
            }
            self.load_selected();
        }

        #[unsafe(method(removeRemote:))]
        fn action_remove(&self, _sender: Option<&AnyObject>) {
            let idx = self
                .ivars()
                .list_popup
                .borrow()
                .as_ref()
                .map(|p| p.indexOfSelectedItem() as usize);
            if let Some(idx) = idx {
                let mut config = self.ivars().config.borrow_mut();
                if idx < config.remotes.len() {
                    config.remotes.remove(idx);
                }
                drop(config);
                self.refresh_list();
                self.load_selected();
            }
        }

        #[unsafe(method(remoteSelected:))]
        fn action_selected(&self, _sender: Option<&AnyObject>) {
            self.load_selected();
        }

        #[unsafe(method(methodChanged:))]
        fn action_method_changed(&self, _sender: Option<&AnyObject>) {
            self.save_current_to_config();
        }

        #[unsafe(method(saveAndClose:))]
        fn action_save(&self, _sender: Option<&AnyObject>) {
            self.save_current_to_config();
            let config = self.ivars().config.borrow();
            if let Err(e) = save_config(&config) {
                log::error!("Failed to save config: {}", e);
            }
            self.close();
        }

        #[unsafe(method(cancelClose:))]
        fn action_cancel(&self, _sender: Option<&AnyObject>) {
            self.close();
        }
    }
);

impl RemotesDialog {
    fn refresh_list(&self) {
        if let Some(popup) = self.ivars().list_popup.borrow().as_ref() {
            popup.removeAllItems();
            let config = self.ivars().config.borrow();
            for remote in &config.remotes {
                popup.addItemWithTitle(&NSString::from_str(&format!(
                    "{} ({})",
                    remote.name, remote.host
                )));
            }
            if config.remotes.is_empty() {
                popup.addItemWithTitle(&NSString::from_str("(no remotes)"));
            }
        }
    }

    fn load_selected(&self) {
        let idx = self
            .ivars()
            .list_popup
            .borrow()
            .as_ref()
            .map(|p| p.indexOfSelectedItem() as usize);
        let config = self.ivars().config.borrow();
        if let Some(idx) = idx {
            if let Some(remote) = config.remotes.get(idx) {
                let ivars = self.ivars();
                set_text(&ivars.name_field, &remote.name);
                set_text(&ivars.host_field, &remote.host);
                if let Some(p) = ivars.method_popup.borrow().as_ref() {
                    p.selectItemAtIndex(match remote.method {
                        ConnectionMethod::Daemon => 0,
                        ConnectionMethod::Mosh => 1,
                    });
                }
                return;
            }
        }
        // Clear all fields
        let ivars = self.ivars();
        set_text(&ivars.name_field, "");
        set_text(&ivars.host_field, "");
        if let Some(p) = ivars.method_popup.borrow().as_ref() {
            p.selectItemAtIndex(0);
        }
    }

    fn save_current_to_config(&self) {
        let idx = self
            .ivars()
            .list_popup
            .borrow()
            .as_ref()
            .map(|p| p.indexOfSelectedItem() as usize);
        if let Some(idx) = idx {
            let mut config = self.ivars().config.borrow_mut();
            if let Some(remote) = config.remotes.get_mut(idx) {
                let ivars = self.ivars();
                if let Some(f) = ivars.name_field.borrow().as_ref() {
                    remote.name = f.stringValue().to_string();
                }
                if let Some(f) = ivars.host_field.borrow().as_ref() {
                    remote.host = f.stringValue().to_string();
                }
                if let Some(p) = ivars.method_popup.borrow().as_ref() {
                    remote.method = match p.indexOfSelectedItem() {
                        1 => ConnectionMethod::Mosh,
                        _ => ConnectionMethod::Daemon,
                    };
                }
            }
            drop(config);
            self.refresh_list();
            if let Some(popup) = self.ivars().list_popup.borrow().as_ref() {
                popup.selectItemAtIndex(idx as isize);
            }
        }
    }
}

/// Set a text field's value from a RefCell<Option<Retained<NSTextField>>>.
fn set_text(field: &RefCell<Option<Retained<NSTextField>>>, value: &str) {
    if let Some(f) = field.borrow().as_ref() {
        f.setStringValue(&NSString::from_str(value));
    }
}

pub fn show_remotes_dialog(mtm: MainThreadMarker, config: Config) {
    let content_rect = NSRect::new(NSPoint::new(300.0, 200.0), NSSize::new(420.0, 480.0));
    let style_mask =
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;

    let this = mtm.alloc::<RemotesDialog>();
    let this = this.set_ivars(RemotesDialogIvars {
        config: RefCell::new(config),
        list_popup: RefCell::new(None),
        name_field: RefCell::new(None),
        host_field: RefCell::new(None),
        method_popup: RefCell::new(None),
    });

    let this: Retained<RemotesDialog> = unsafe {
        msg_send![
            super(this),
            initWithContentRect: content_rect,
            styleMask: style_mask,
            backing: 2u64,
            defer: false
        ]
    };

    this.setTitle(&NSString::from_str("Manage Remotes"));
    this.setMinSize(NSSize::new(350.0, 220.0));
    unsafe { this.setReleasedWhenClosed(false) };

    // Build UI
    let main_stack = unsafe { NSStackView::new(mtm) };
    main_stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    main_stack.setSpacing(12.0);
    main_stack.setAlignment(NSLayoutAttribute::Leading);
    unsafe {
        main_stack.setEdgeInsets(objc2_foundation::NSEdgeInsets {
            top: 12.0,
            left: 12.0,
            bottom: 12.0,
            right: 12.0,
        });
    }

    // Top row: popup + add/remove buttons
    let top_row = unsafe { NSStackView::new(mtm) };
    top_row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    top_row.setSpacing(8.0);

    let popup = unsafe { objc2_app_kit::NSPopUpButton::new(mtm) };
    unsafe {
        popup.setTarget(Some(&*this));
        popup.setAction(Some(sel!(remoteSelected:)));
    }
    top_row.addView_inGravity(&popup, NSStackViewGravity::Leading);

    let add_btn = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("+"),
            Some(&*this),
            Some(sel!(addRemote:)),
            mtm,
        )
    };
    let remove_btn = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("-"),
            Some(&*this),
            Some(sel!(removeRemote:)),
            mtm,
        )
    };
    top_row.addView_inGravity(&add_btn, NSStackViewGravity::Leading);
    top_row.addView_inGravity(&remove_btn, NSStackViewGravity::Leading);

    main_stack.addView_inGravity(&top_row, NSStackViewGravity::Top);

    // Name field
    let name_row = unsafe { NSStackView::new(mtm) };
    name_row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    name_row.setSpacing(8.0);
    let name_label = unsafe { NSTextField::labelWithString(&NSString::from_str("Name:"), mtm) };
    unsafe {
        let _: () = msg_send![&*name_label, setPreferredMaxLayoutWidth: 60.0f64];
    }
    let name_field = unsafe {
        let f = NSTextField::new(mtm);
        f.setPlaceholderString(Some(&NSString::from_str("my-server")));
        f.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*this)));
        f
    };
    name_row.addView_inGravity(&name_label, NSStackViewGravity::Leading);
    name_row.addView_inGravity(&name_field, NSStackViewGravity::Leading);
    main_stack.addView_inGravity(&name_row, NSStackViewGravity::Top);

    // Host field
    let host_row = unsafe { NSStackView::new(mtm) };
    host_row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    host_row.setSpacing(8.0);
    let host_label = unsafe { NSTextField::labelWithString(&NSString::from_str("Host:"), mtm) };
    unsafe {
        let _: () = msg_send![&*host_label, setPreferredMaxLayoutWidth: 60.0f64];
    }
    let host_field = unsafe {
        let f = NSTextField::new(mtm);
        f.setPlaceholderString(Some(&NSString::from_str("user@hostname")));
        f.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*this)));
        f
    };
    host_row.addView_inGravity(&host_label, NSStackViewGravity::Leading);
    host_row.addView_inGravity(&host_field, NSStackViewGravity::Leading);
    main_stack.addView_inGravity(&host_row, NSStackViewGravity::Top);

    // Method popup
    let method_row = unsafe { NSStackView::new(mtm) };
    method_row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    method_row.setSpacing(8.0);
    let method_label = unsafe { NSTextField::labelWithString(&NSString::from_str("Method:"), mtm) };
    unsafe {
        let _: () = msg_send![&*method_label, setPreferredMaxLayoutWidth: 60.0f64];
    }
    let method_popup = unsafe {
        let p = objc2_app_kit::NSPopUpButton::new(mtm);
        p.addItemWithTitle(&NSString::from_str("ctermd"));
        p.addItemWithTitle(&NSString::from_str("Mosh"));
        p.setTarget(Some(&*this));
        p.setAction(Some(sel!(methodChanged:)));
        p
    };
    method_row.addView_inGravity(&method_label, NSStackViewGravity::Leading);
    method_row.addView_inGravity(&method_popup, NSStackViewGravity::Leading);
    main_stack.addView_inGravity(&method_row, NSStackViewGravity::Top);

    // Bottom row: Cancel / Save
    let bottom_row = unsafe { NSStackView::new(mtm) };
    bottom_row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    bottom_row.setSpacing(8.0);

    let cancel_btn = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("Cancel"),
            Some(&*this),
            Some(sel!(cancelClose:)),
            mtm,
        )
    };
    let save_btn = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("Save"),
            Some(&*this),
            Some(sel!(saveAndClose:)),
            mtm,
        )
    };

    bottom_row.addView_inGravity(&cancel_btn, NSStackViewGravity::Trailing);
    bottom_row.addView_inGravity(&save_btn, NSStackViewGravity::Trailing);
    main_stack.addView_inGravity(&bottom_row, NSStackViewGravity::Bottom);

    // Store references
    *this.ivars().list_popup.borrow_mut() = Some(popup);
    *this.ivars().name_field.borrow_mut() = Some(name_field);
    *this.ivars().host_field.borrow_mut() = Some(host_field);
    *this.ivars().method_popup.borrow_mut() = Some(method_popup);

    // Populate list
    this.refresh_list();
    if !this.ivars().config.borrow().remotes.is_empty() {
        if let Some(popup) = this.ivars().list_popup.borrow().as_ref() {
            popup.selectItemAtIndex(0);
        }
        this.load_selected();
    }

    this.setContentView(Some(&main_stack));
    this.makeKeyAndOrderFront(None);
}
