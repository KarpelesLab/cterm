//! Unix Shells sign-in dialog for macOS.

use std::cell::RefCell;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, sel, MainThreadOnly};
use objc2_app_kit::{
    NSButton, NSProgressIndicator, NSStackView, NSTextField, NSWindow, NSWindowDelegate,
    NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use cterm_app::unixshells::DeviceService;

pub struct UnixShellsDialogIvars {
    device_service: Arc<DeviceService>,
    username_field: RefCell<Option<Retained<NSTextField>>>,
    status_label: RefCell<Option<Retained<NSTextField>>>,
    spinner: RefCell<Option<Retained<NSProgressIndicator>>>,
    signin_button: RefCell<Option<Retained<NSButton>>>,
}

define_class!(
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "UnixShellsDialog"]
    #[ivars = UnixShellsDialogIvars]
    pub struct UnixShellsDialog;

    unsafe impl NSObjectProtocol for UnixShellsDialog {}

    unsafe impl NSWindowDelegate for UnixShellsDialog {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {}
    }

    impl UnixShellsDialog {
        #[unsafe(method(doSignIn:))]
        fn action_sign_in(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let username = {
                let field = self.ivars().username_field.borrow();
                field.as_ref().map(|f| f.stringValue().to_string()).unwrap_or_default()
            };
            let username = username.trim().to_lowercase();
            if username.is_empty() {
                return;
            }

            // Show waiting state
            if let Some(ref label) = *self.ivars().status_label.borrow() {
                label.setStringValue(&NSString::from_str("Check your email to approve this device..."));
            }
            if let Some(ref spinner) = *self.ivars().spinner.borrow() {
                spinner.startAnimation(None);
                spinner.setHidden(false);
            }
            if let Some(ref btn) = *self.ivars().signin_button.borrow() {
                btn.setEnabled(false);
            }
            if let Some(ref field) = *self.ivars().username_field.borrow() {
                field.setEnabled(false);
            }

            // Start login in background
            let ds = self.ivars().device_service.clone();
            std::thread::spawn(move || {
                if let Err(e) = ds.start_login(&username) {
                    log::error!("Unix Shells login failed: {}", e);
                }
            });

            // Poll for completion
            let ds = self.ivars().device_service.clone();
            let status_label = self.ivars().status_label.borrow().clone();
            let window: Retained<NSWindow> = self.retain().into_super();

            let last_version = std::cell::Cell::new(ds.version.load(std::sync::atomic::Ordering::Relaxed));

            // Use a dispatch timer for polling
            let timer_block = block2::RcBlock::new(move || {
                let current = ds.version.load(std::sync::atomic::Ordering::Relaxed);
                if current == last_version.get() {
                    return;
                }
                last_version.set(current);

                match ds.login_state() {
                    cterm_app::unixshells::LoginState::LoggedIn { ref username } => {
                        if let Some(ref label) = status_label {
                            label.setStringValue(&NSString::from_str(
                                &format!("Signed in as {}", username),
                            ));
                        }
                        window.close();
                    }
                    cterm_app::unixshells::LoginState::LoggedOut => {
                        if let Some(ref label) = status_label {
                            let msg = ds.last_error().unwrap_or("Sign in failed".to_string());
                            label.setStringValue(&NSString::from_str(&msg));
                        }
                    }
                    _ => {}
                }
            });

            unsafe {
                let timer = objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(
                    0.5,
                    true,
                    &timer_block,
                );
                // Timer will fire until window closes
                let _ = timer;
            }
        }

        #[unsafe(method(doCancel:))]
        fn action_cancel(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.close();
        }
    }
);

/// Show the Unix Shells sign-in dialog.
pub fn show_unixshells_dialog(mtm: MainThreadMarker, device_service: Arc<DeviceService>) {
    let content_rect = NSRect::new(NSPoint::new(400.0, 300.0), NSSize::new(400.0, 200.0));
    let style_mask = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;

    let this = mtm.alloc::<UnixShellsDialog>();
    let this = this.set_ivars(UnixShellsDialogIvars {
        device_service,
        username_field: RefCell::new(None),
        status_label: RefCell::new(None),
        spinner: RefCell::new(None),
        signin_button: RefCell::new(None),
    });

    let this: Retained<UnixShellsDialog> = unsafe {
        msg_send![
            super(this),
            initWithContentRect: content_rect,
            styleMask: style_mask,
            backing: 2u64,
            defer: false
        ]
    };

    this.setTitle(&NSString::from_str("Unix Shells - Sign In"));
    unsafe { this.setReleasedWhenClosed(false) };
    this.setDelegate(Some(ProtocolObject::from_ref(&*this)));

    // Build UI
    let stack = unsafe {
        let stack = NSStackView::new(mtm);
        stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Vertical);
        stack.setAlignment(objc2_app_kit::NSLayoutAttribute::CenterX);
        stack.setSpacing(12.0);
        stack.setEdgeInsets(objc2_foundation::NSEdgeInsets {
            top: 20.0,
            left: 20.0,
            bottom: 20.0,
            right: 20.0,
        });
        stack
    };

    // Title
    let title = unsafe {
        let label = NSTextField::wrappingLabelWithString(
            &NSString::from_str("Sign in to Unix Shells"),
            mtm,
        );
        label.setAlignment(objc2_app_kit::NSTextAlignment::Center);
        label
    };
    unsafe { stack.addArrangedSubview(&title) };

    // Username field
    let username_field = NSTextField::new(mtm);
    username_field.setPlaceholderString(Some(&NSString::from_str("username")));
    *this.ivars().username_field.borrow_mut() = Some(username_field.clone());
    unsafe { stack.addArrangedSubview(&username_field) };

    // Status label
    let status_label = unsafe {
        let label = NSTextField::wrappingLabelWithString(&NSString::from_str(""), mtm);
        label.setAlignment(objc2_app_kit::NSTextAlignment::Center);
        label
    };
    *this.ivars().status_label.borrow_mut() = Some(status_label.clone());
    unsafe { stack.addArrangedSubview(&status_label) };

    // Spinner (hidden initially)
    let spinner = NSProgressIndicator::new(mtm);
    unsafe {
        spinner.setStyle(objc2_app_kit::NSProgressIndicatorStyle::Spinning);
        spinner.setDisplayedWhenStopped(false);
    }
    *this.ivars().spinner.borrow_mut() = Some(spinner.clone());
    unsafe { stack.addArrangedSubview(&spinner) };

    // Buttons
    let button_stack = unsafe {
        let s = NSStackView::new(mtm);
        s.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Horizontal);
        s.setSpacing(8.0);
        s
    };

    let cancel_btn = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("Cancel"),
            Some(&*this),
            Some(sel!(doCancel:)),
            mtm,
        )
    };
    unsafe { button_stack.addArrangedSubview(&cancel_btn) };

    let signin_btn = unsafe {
        let btn = NSButton::buttonWithTitle_target_action(
            &NSString::from_str("Sign In"),
            Some(&*this),
            Some(sel!(doSignIn:)),
            mtm,
        );
        btn.setKeyEquivalent(&NSString::from_str("\r"));
        btn
    };
    *this.ivars().signin_button.borrow_mut() = Some(signin_btn.clone());
    unsafe { button_stack.addArrangedSubview(&signin_btn) };

    unsafe { stack.addArrangedSubview(&button_stack) };

    this.setContentView(Some(&stack));
    this.center();
    this.makeKeyAndOrderFront(None);
}
