//! Preferences window for macOS
//!
//! Implements a native preferences window with tabs for different settings categories.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSButton, NSPopUpButton, NSSlider, NSStackView, NSTabView, NSTabViewItem, NSTextField,
    NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use std::time::{SystemTime, UNIX_EPOCH};

use cterm_app::config::{
    config_dir, save_config, Config, CursorStyleConfig, NewTabPosition, TabBarPosition,
    TabBarVisibility, ToolShortcutEntry,
};
use cterm_app::{git_sync, PullResult};

/// Format a Unix timestamp as a human-readable relative time
fn format_timestamp(ts: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let diff = now - ts;

    if diff < 60 {
        "Just now".to_string()
    } else if diff < 3600 {
        let mins = diff / 60;
        format!("{} minute{} ago", mins, if mins == 1 { "" } else { "s" })
    } else if diff < 86400 {
        let hours = diff / 3600;
        format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
    } else if diff < 604800 {
        let days = diff / 86400;
        format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
    } else {
        let weeks = diff / 604800;
        format!("{} week{} ago", weeks, if weeks == 1 { "" } else { "s" })
    }
}

/// Preferences window ivars
pub struct PreferencesWindowIvars {
    config: RefCell<Config>,
    on_save: RefCell<Option<Box<dyn Fn(Config)>>>,
    // General tab controls
    scrollback_field: RefCell<Option<Retained<NSTextField>>>,
    confirm_close_checkbox: RefCell<Option<Retained<NSButton>>>,
    copy_on_select_checkbox: RefCell<Option<Retained<NSButton>>>,
    // Appearance tab controls
    theme_popup: RefCell<Option<Retained<NSPopUpButton>>>,
    font_field: RefCell<Option<Retained<NSTextField>>>,
    font_size_field: RefCell<Option<Retained<NSTextField>>>,
    cursor_popup: RefCell<Option<Retained<NSPopUpButton>>>,
    cursor_blink_checkbox: RefCell<Option<Retained<NSButton>>>,
    opacity_slider: RefCell<Option<Retained<NSSlider>>>,
    bold_bright_checkbox: RefCell<Option<Retained<NSButton>>>,
    // Tabs tab controls
    show_tab_bar_popup: RefCell<Option<Retained<NSPopUpButton>>>,
    tab_position_popup: RefCell<Option<Retained<NSPopUpButton>>>,
    new_tab_popup: RefCell<Option<Retained<NSPopUpButton>>>,
    show_close_checkbox: RefCell<Option<Retained<NSButton>>>,
    // Tools tab controls
    tool_entries_stack: RefCell<Option<Retained<NSStackView>>>,
    tool_entries: RefCell<
        Vec<(
            Retained<NSTextField>,
            Retained<NSTextField>,
            Retained<NSTextField>,
        )>,
    >,
    // Git Sync tab controls
    git_remote_field: RefCell<Option<Retained<NSTextField>>>,
    git_status_label: RefCell<Option<Retained<NSTextField>>>,
    git_branch_label: RefCell<Option<Retained<NSTextField>>>,
    git_last_sync_label: RefCell<Option<Retained<NSTextField>>>,
    git_changes_label: RefCell<Option<Retained<NSTextField>>>,
}

define_class!(
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "PreferencesWindow"]
    #[ivars = PreferencesWindowIvars]
    pub struct PreferencesWindow;

    unsafe impl NSObjectProtocol for PreferencesWindow {}

    unsafe impl NSWindowDelegate for PreferencesWindow {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            log::debug!("Preferences window closing");
        }
    }

    // Button action handlers
    impl PreferencesWindow {
        #[unsafe(method(savePreferences:))]
        fn action_save(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.collect_and_save();
            self.close();
        }

        #[unsafe(method(cancelPreferences:))]
        fn action_cancel(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.close();
        }

        #[unsafe(method(applyPreferences:))]
        fn action_apply(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.collect_and_save();
        }

        #[unsafe(method(addToolEntry:))]
        fn action_add_tool_entry(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            self.add_tool_entry_row(mtm, "", "", "");
        }

        #[unsafe(method(resetToolDefaults:))]
        fn action_reset_tool_defaults(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            // Clear existing entries
            self.ivars().tool_entries.borrow_mut().clear();
            if let Some(ref stack) = *self.ivars().tool_entries_stack.borrow() {
                // Remove all arranged subviews (entry rows)
                let subviews = stack.arrangedSubviews();
                for view in subviews.iter() {
                    stack.removeArrangedSubview(&view);
                    unsafe {
                        view.removeFromSuperview();
                    }
                }
            }
            // Add defaults
            for entry in cterm_app::config::default_tool_shortcuts() {
                self.add_tool_entry_row(mtm, &entry.name, &entry.command, &entry.args.join(" "));
            }
        }

        #[unsafe(method(syncNow:))]
        fn action_sync_now(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.perform_sync_now();
        }
    }
);

impl PreferencesWindow {
    pub fn new(
        mtm: MainThreadMarker,
        config: &Config,
        on_save: impl Fn(Config) + 'static,
    ) -> Retained<Self> {
        let content_rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(500.0, 400.0));

        let style_mask = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable;

        // Allocate and initialize
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(PreferencesWindowIvars {
            config: RefCell::new(config.clone()),
            on_save: RefCell::new(Some(Box::new(on_save))),
            scrollback_field: RefCell::new(None),
            confirm_close_checkbox: RefCell::new(None),
            copy_on_select_checkbox: RefCell::new(None),
            theme_popup: RefCell::new(None),
            font_field: RefCell::new(None),
            font_size_field: RefCell::new(None),
            cursor_popup: RefCell::new(None),
            cursor_blink_checkbox: RefCell::new(None),
            opacity_slider: RefCell::new(None),
            bold_bright_checkbox: RefCell::new(None),
            show_tab_bar_popup: RefCell::new(None),
            tab_position_popup: RefCell::new(None),
            new_tab_popup: RefCell::new(None),
            show_close_checkbox: RefCell::new(None),
            tool_entries_stack: RefCell::new(None),
            tool_entries: RefCell::new(Vec::new()),
            git_remote_field: RefCell::new(None),
            git_status_label: RefCell::new(None),
            git_branch_label: RefCell::new(None),
            git_last_sync_label: RefCell::new(None),
            git_changes_label: RefCell::new(None),
        });

        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: content_rect,
                styleMask: style_mask,
                backing: 2u64,
                defer: false
            ]
        };

        this.setTitle(&NSString::from_str("Preferences"));
        // Prevent macOS from releasing window on close (we manage lifetime)
        unsafe { this.setReleasedWhenClosed(false) };
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));

        // Create the tab view
        this.setup_ui(mtm, config);

        this
    }

    fn setup_ui(&self, mtm: MainThreadMarker, config: &Config) {
        // Create a container view for manual layout
        let container = unsafe {
            let view = objc2_app_kit::NSView::new(mtm);
            view.setTranslatesAutoresizingMaskIntoConstraints(false);
            view
        };

        // Create tab view
        let tab_view = NSTabView::new(mtm);
        unsafe {
            tab_view.setTranslatesAutoresizingMaskIntoConstraints(false);
        }

        // Add tabs
        let general_tab = self.create_general_tab(mtm, config);
        tab_view.addTabViewItem(&general_tab);

        let appearance_tab = self.create_appearance_tab(mtm, config);
        tab_view.addTabViewItem(&appearance_tab);

        let tabs_tab = self.create_tabs_tab(mtm, config);
        tab_view.addTabViewItem(&tabs_tab);

        let tools_tab = self.create_tools_tab(mtm);
        tab_view.addTabViewItem(&tools_tab);

        let git_sync_tab = self.create_git_sync_tab(mtm);
        tab_view.addTabViewItem(&git_sync_tab);

        unsafe {
            container.addSubview(&tab_view);
        }

        // Create button row
        let button_stack = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Horizontal);
            stack.setSpacing(8.0);
            stack.setTranslatesAutoresizingMaskIntoConstraints(false);
            stack
        };

        // Spacer to push buttons right
        let spacer = NSTextField::new(mtm);
        spacer.setEditable(false);
        spacer.setBordered(false);
        spacer.setDrawsBackground(false);
        spacer.setStringValue(&NSString::from_str(""));
        unsafe {
            let _: () =
                msg_send![&spacer, setContentHuggingPriority: 1.0_f32, forOrientation: 0i64];
        }
        unsafe {
            button_stack.addArrangedSubview(&spacer);
        }

        // Cancel button
        let cancel_btn = unsafe {
            let btn = NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Cancel"),
                Some(&*self),
                Some(sel!(cancelPreferences:)),
                mtm,
            );
            btn
        };
        unsafe {
            button_stack.addArrangedSubview(&cancel_btn);
        }

        // Apply button
        let apply_btn = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Apply"),
                Some(&*self),
                Some(sel!(applyPreferences:)),
                mtm,
            )
        };
        unsafe {
            button_stack.addArrangedSubview(&apply_btn);
        }

        // OK button
        let ok_btn = unsafe {
            let btn = NSButton::buttonWithTitle_target_action(
                &NSString::from_str("OK"),
                Some(&*self),
                Some(sel!(savePreferences:)),
                mtm,
            );
            btn.setKeyEquivalent(&NSString::from_str("\r")); // Enter key
            btn
        };
        unsafe {
            button_stack.addArrangedSubview(&ok_btn);
        }

        unsafe {
            container.addSubview(&button_stack);
        }

        // Set up Auto Layout constraints
        unsafe {
            use objc2_app_kit::NSLayoutConstraint;

            // Tab view: pin to top, left, right with margins
            let c1 = tab_view
                .topAnchor()
                .constraintEqualToAnchor_constant(&container.topAnchor(), 12.0);
            let c2 = tab_view
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&container.leadingAnchor(), 12.0);
            let c3 = tab_view
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&container.trailingAnchor(), -12.0);

            // Button stack: pin to bottom, left, right with margins
            let c4 = button_stack
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&container.leadingAnchor(), 12.0);
            let c5 = button_stack
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&container.trailingAnchor(), -12.0);
            let c6 = button_stack
                .bottomAnchor()
                .constraintEqualToAnchor_constant(&container.bottomAnchor(), -12.0);

            // Connect tab view bottom to button stack top
            let c7 = tab_view
                .bottomAnchor()
                .constraintEqualToAnchor_constant(&button_stack.topAnchor(), -12.0);

            NSLayoutConstraint::activateConstraints(&objc2_foundation::NSArray::from_slice(&[
                &*c1, &*c2, &*c3, &*c4, &*c5, &*c6, &*c7,
            ]));
        }

        self.setContentView(Some(&container));
    }

    fn create_general_tab(
        &self,
        mtm: MainThreadMarker,
        config: &Config,
    ) -> Retained<NSTabViewItem> {
        let tab = NSTabViewItem::new();
        tab.setLabel(&NSString::from_str("General"));

        let stack = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Vertical);
            stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
            stack.setSpacing(12.0);
            stack.setEdgeInsets(objc2_foundation::NSEdgeInsets {
                top: 16.0,
                left: 16.0,
                bottom: 16.0,
                right: 16.0,
            });
            stack
        };

        // Scrollback lines
        let scrollback_row = self.create_label_field_row(
            mtm,
            "Scrollback lines:",
            &config.general.scrollback_lines.to_string(),
        );
        *self.ivars().scrollback_field.borrow_mut() = Some(scrollback_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&scrollback_row.0);
        }

        // Confirm close with running processes
        let confirm_checkbox = self.create_checkbox(
            mtm,
            "Confirm close with running processes",
            config.general.confirm_close_with_running,
        );
        *self.ivars().confirm_close_checkbox.borrow_mut() = Some(confirm_checkbox.clone());
        unsafe {
            stack.addArrangedSubview(&confirm_checkbox);
        }

        // Copy on select
        let copy_checkbox =
            self.create_checkbox(mtm, "Copy on select", config.general.copy_on_select);
        *self.ivars().copy_on_select_checkbox.borrow_mut() = Some(copy_checkbox.clone());
        unsafe {
            stack.addArrangedSubview(&copy_checkbox);
        }

        tab.setView(Some(&stack));
        tab
    }

    fn create_appearance_tab(
        &self,
        mtm: MainThreadMarker,
        config: &Config,
    ) -> Retained<NSTabViewItem> {
        let tab = NSTabViewItem::new();
        tab.setLabel(&NSString::from_str("Appearance"));

        let stack = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Vertical);
            stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
            stack.setSpacing(12.0);
            stack.setEdgeInsets(objc2_foundation::NSEdgeInsets {
                top: 16.0,
                left: 16.0,
                bottom: 16.0,
                right: 16.0,
            });
            stack
        };

        // Theme popup
        let themes = [
            ("dark", "Default Dark"),
            ("light", "Default Light"),
            ("tokyo_night", "Tokyo Night"),
            ("dracula", "Dracula"),
            ("nord", "Nord"),
        ];
        let theme_row =
            self.create_label_popup_row(mtm, "Theme:", &themes, &config.appearance.theme);
        *self.ivars().theme_popup.borrow_mut() = Some(theme_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&theme_row.0);
        }

        // Font
        let font_row = self.create_label_field_row(mtm, "Font:", &config.appearance.font.family);
        *self.ivars().font_field.borrow_mut() = Some(font_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&font_row.0);
        }

        // Font size
        let size_row = self.create_label_field_row(
            mtm,
            "Font size:",
            &config.appearance.font.size.to_string(),
        );
        *self.ivars().font_size_field.borrow_mut() = Some(size_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&size_row.0);
        }

        // Cursor style
        let cursor_styles = [
            ("block", "Block"),
            ("underline", "Underline"),
            ("bar", "Bar"),
        ];
        let cursor_id = match config.appearance.cursor_style {
            CursorStyleConfig::Block => "block",
            CursorStyleConfig::Underline => "underline",
            CursorStyleConfig::Bar => "bar",
        };
        let cursor_row =
            self.create_label_popup_row(mtm, "Cursor style:", &cursor_styles, cursor_id);
        *self.ivars().cursor_popup.borrow_mut() = Some(cursor_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&cursor_row.0);
        }

        // Cursor blink
        let blink_checkbox =
            self.create_checkbox(mtm, "Cursor blink", config.appearance.cursor_blink);
        *self.ivars().cursor_blink_checkbox.borrow_mut() = Some(blink_checkbox.clone());
        unsafe {
            stack.addArrangedSubview(&blink_checkbox);
        }

        // Opacity slider
        let opacity_row =
            self.create_label_slider_row(mtm, "Opacity:", config.appearance.opacity, 0.0, 1.0);
        *self.ivars().opacity_slider.borrow_mut() = Some(opacity_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&opacity_row.0);
        }

        // Bold is bright
        let bold_checkbox = self.create_checkbox(
            mtm,
            "Bold text uses bright colors",
            config.appearance.bold_is_bright,
        );
        *self.ivars().bold_bright_checkbox.borrow_mut() = Some(bold_checkbox.clone());
        unsafe {
            stack.addArrangedSubview(&bold_checkbox);
        }

        tab.setView(Some(&stack));
        tab
    }

    fn create_tabs_tab(&self, mtm: MainThreadMarker, config: &Config) -> Retained<NSTabViewItem> {
        let tab = NSTabViewItem::new();
        tab.setLabel(&NSString::from_str("Tabs"));

        let stack = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Vertical);
            stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
            stack.setSpacing(12.0);
            stack.setEdgeInsets(objc2_foundation::NSEdgeInsets {
                top: 16.0,
                left: 16.0,
                bottom: 16.0,
                right: 16.0,
            });
            stack
        };

        // Show tab bar
        let show_options = [
            ("always", "Always"),
            ("multiple", "When multiple tabs"),
            ("never", "Never"),
        ];
        let show_id = match config.tabs.show_tab_bar {
            TabBarVisibility::Always => "always",
            TabBarVisibility::Multiple => "multiple",
            TabBarVisibility::Never => "never",
        };
        let show_row = self.create_label_popup_row(mtm, "Show tab bar:", &show_options, show_id);
        *self.ivars().show_tab_bar_popup.borrow_mut() = Some(show_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&show_row.0);
        }

        // Tab bar position
        let position_options = [("top", "Top"), ("bottom", "Bottom")];
        let position_id = match config.tabs.tab_bar_position {
            TabBarPosition::Top => "top",
            TabBarPosition::Bottom => "bottom",
        };
        let position_row =
            self.create_label_popup_row(mtm, "Tab bar position:", &position_options, position_id);
        *self.ivars().tab_position_popup.borrow_mut() = Some(position_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&position_row.0);
        }

        // New tab position
        let new_options = [("end", "At end"), ("after_current", "After current")];
        let new_id = match config.tabs.new_tab_position {
            NewTabPosition::End => "end",
            NewTabPosition::AfterCurrent => "after_current",
        };
        let new_row = self.create_label_popup_row(mtm, "New tab position:", &new_options, new_id);
        *self.ivars().new_tab_popup.borrow_mut() = Some(new_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&new_row.0);
        }

        // Show close button
        let close_checkbox = self.create_checkbox(
            mtm,
            "Show close button on tabs",
            config.tabs.show_close_button,
        );
        *self.ivars().show_close_checkbox.borrow_mut() = Some(close_checkbox.clone());
        unsafe {
            stack.addArrangedSubview(&close_checkbox);
        }

        tab.setView(Some(&stack));
        tab
    }

    fn create_tools_tab(&self, mtm: MainThreadMarker) -> Retained<NSTabViewItem> {
        let tab = NSTabViewItem::new();
        tab.setLabel(&NSString::from_str("Tools"));

        let outer_stack = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Vertical);
            stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
            stack.setSpacing(12.0);
            stack.setEdgeInsets(objc2_foundation::NSEdgeInsets {
                top: 16.0,
                left: 16.0,
                bottom: 16.0,
                right: 16.0,
            });
            stack
        };

        // Header label
        let header =
            NSTextField::labelWithString(&NSString::from_str("External Tool Shortcuts"), mtm);
        unsafe {
            outer_stack.addArrangedSubview(&header);
        }

        // Column headers
        let headers_row = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Horizontal);
            stack.setSpacing(8.0);
            stack
        };
        let name_header = NSTextField::labelWithString(&NSString::from_str("Name"), mtm);
        let cmd_header = NSTextField::labelWithString(&NSString::from_str("Command"), mtm);
        let args_header = NSTextField::labelWithString(&NSString::from_str("Args"), mtm);
        unsafe {
            let size = objc2_foundation::NSSize::new(120.0, 17.0);
            let _: () = msg_send![&name_header, setFrameSize: size];
            let _: () = msg_send![&cmd_header, setFrameSize: size];
            let _: () = msg_send![&args_header, setFrameSize: size];
            headers_row.addArrangedSubview(&name_header);
            headers_row.addArrangedSubview(&cmd_header);
            headers_row.addArrangedSubview(&args_header);
        }
        unsafe {
            outer_stack.addArrangedSubview(&headers_row);
        }

        // Entries stack
        let entries_stack = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Vertical);
            stack.setSpacing(4.0);
            stack
        };
        *self.ivars().tool_entries_stack.borrow_mut() = Some(entries_stack.clone());
        unsafe {
            outer_stack.addArrangedSubview(&entries_stack);
        }

        // Load existing entries
        let shortcuts = cterm_app::config::load_tool_shortcuts().unwrap_or_default();
        for entry in &shortcuts {
            self.add_tool_entry_row(mtm, &entry.name, &entry.command, &entry.args.join(" "));
        }

        // Button row
        let button_row = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Horizontal);
            stack.setSpacing(8.0);
            stack
        };

        let add_btn = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Add"),
                Some(&*self),
                Some(sel!(addToolEntry:)),
                mtm,
            )
        };
        let reset_btn = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Reset to Defaults"),
                Some(&*self),
                Some(sel!(resetToolDefaults:)),
                mtm,
            )
        };
        unsafe {
            button_row.addArrangedSubview(&add_btn);
            button_row.addArrangedSubview(&reset_btn);
        }
        unsafe {
            outer_stack.addArrangedSubview(&button_row);
        }

        tab.setView(Some(&outer_stack));
        tab
    }

    fn add_tool_entry_row(&self, mtm: MainThreadMarker, name: &str, command: &str, args: &str) {
        let row = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Horizontal);
            stack.setSpacing(8.0);
            stack
        };

        let name_field = NSTextField::new(mtm);
        name_field.setStringValue(&NSString::from_str(name));
        name_field.setEditable(true);
        name_field.setBordered(true);
        name_field.setDrawsBackground(true);
        name_field.setPlaceholderString(Some(&NSString::from_str("Name")));
        unsafe {
            let size = objc2_foundation::NSSize::new(120.0, 22.0);
            let _: () = msg_send![&name_field, setFrameSize: size];
        }

        let cmd_field = NSTextField::new(mtm);
        cmd_field.setStringValue(&NSString::from_str(command));
        cmd_field.setEditable(true);
        cmd_field.setBordered(true);
        cmd_field.setDrawsBackground(true);
        cmd_field.setPlaceholderString(Some(&NSString::from_str("Command")));
        unsafe {
            let size = objc2_foundation::NSSize::new(120.0, 22.0);
            let _: () = msg_send![&cmd_field, setFrameSize: size];
        }

        let args_field = NSTextField::new(mtm);
        args_field.setStringValue(&NSString::from_str(args));
        args_field.setEditable(true);
        args_field.setBordered(true);
        args_field.setDrawsBackground(true);
        args_field.setPlaceholderString(Some(&NSString::from_str("Arguments")));
        unsafe {
            let size = objc2_foundation::NSSize::new(120.0, 22.0);
            let _: () = msg_send![&args_field, setFrameSize: size];
        }

        unsafe {
            row.addArrangedSubview(&name_field);
            row.addArrangedSubview(&cmd_field);
            row.addArrangedSubview(&args_field);
        }

        if let Some(ref stack) = *self.ivars().tool_entries_stack.borrow() {
            unsafe {
                stack.addArrangedSubview(&row);
            }
        }

        self.ivars()
            .tool_entries
            .borrow_mut()
            .push((name_field, cmd_field, args_field));
    }

    fn create_git_sync_tab(&self, mtm: MainThreadMarker) -> Retained<NSTabViewItem> {
        let tab = NSTabViewItem::new();
        tab.setLabel(&NSString::from_str("Git Sync"));

        let stack = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Vertical);
            stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
            stack.setSpacing(12.0);
            stack.setEdgeInsets(objc2_foundation::NSEdgeInsets {
                top: 16.0,
                left: 16.0,
                bottom: 16.0,
                right: 16.0,
            });
            stack
        };

        // Get sync status
        let status = config_dir()
            .map(|dir| git_sync::get_sync_status(&dir))
            .unwrap_or_default();

        // Remote URL section
        let remote_header =
            NSTextField::labelWithString(&NSString::from_str("Remote Repository"), mtm);
        unsafe {
            stack.addArrangedSubview(&remote_header);
        }

        let existing_remote = status.remote_url.clone().unwrap_or_default();
        let git_remote_row = self.create_label_field_row(mtm, "Git Remote URL:", &existing_remote);
        git_remote_row
            .1
            .setPlaceholderString(Some(&NSString::from_str(
                "https://github.com/user/config.git",
            )));
        *self.ivars().git_remote_field.borrow_mut() = Some(git_remote_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&git_remote_row.0);
        }

        // Separator
        let separator = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        unsafe {
            stack.addArrangedSubview(&separator);
        }

        // Status section
        let status_header = NSTextField::labelWithString(&NSString::from_str("Sync Status"), mtm);
        unsafe {
            stack.addArrangedSubview(&status_header);
        }

        // Status
        let status_text = if !status.is_repo {
            "Not initialized"
        } else if status.remote_url.is_none() {
            "No remote configured"
        } else {
            "Configured"
        };
        let status_row = self.create_label_field_row(mtm, "Status:", status_text);
        status_row.1.setEditable(false);
        status_row.1.setDrawsBackground(false);
        status_row.1.setBordered(false);
        *self.ivars().git_status_label.borrow_mut() = Some(status_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&status_row.0);
        }

        // Branch
        let branch_text = status.branch.clone().unwrap_or_else(|| "-".to_string());
        let branch_row = self.create_label_field_row(mtm, "Branch:", &branch_text);
        branch_row.1.setEditable(false);
        branch_row.1.setDrawsBackground(false);
        branch_row.1.setBordered(false);
        *self.ivars().git_branch_label.borrow_mut() = Some(branch_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&branch_row.0);
        }

        // Last sync
        let last_sync_text = if let Some(ts) = status.last_commit_time {
            format_timestamp(ts)
        } else {
            "-".to_string()
        };
        let last_sync_row = self.create_label_field_row(mtm, "Last sync:", &last_sync_text);
        last_sync_row.1.setEditable(false);
        last_sync_row.1.setDrawsBackground(false);
        last_sync_row.1.setBordered(false);
        *self.ivars().git_last_sync_label.borrow_mut() = Some(last_sync_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&last_sync_row.0);
        }

        // Changes status
        let changes_text = if status.has_local_changes {
            "Uncommitted changes"
        } else if status.commits_ahead > 0 && status.commits_behind > 0 {
            "Diverged from remote"
        } else if status.commits_ahead > 0 {
            "Ahead of remote"
        } else if status.commits_behind > 0 {
            "Behind remote"
        } else {
            "Up to date"
        };
        let changes_row = self.create_label_field_row(mtm, "Changes:", changes_text);
        changes_row.1.setEditable(false);
        changes_row.1.setDrawsBackground(false);
        changes_row.1.setBordered(false);
        *self.ivars().git_changes_label.borrow_mut() = Some(changes_row.1.clone());
        unsafe {
            stack.addArrangedSubview(&changes_row.0);
        }

        // Separator
        let separator2 = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        unsafe {
            stack.addArrangedSubview(&separator2);
        }

        // Sync Now button
        let sync_btn = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Sync Now"),
                Some(&*self),
                Some(sel!(syncNow:)),
                mtm,
            )
        };
        unsafe {
            stack.addArrangedSubview(&sync_btn);
        }

        tab.setView(Some(&stack));
        tab
    }

    fn perform_sync_now(&self) {
        let Some(dir) = config_dir() else {
            log::error!("No config directory found");
            return;
        };

        // First, check if we need to initialize with remote
        if let Some(ref field) = *self.ivars().git_remote_field.borrow() {
            let remote_url = field.stringValue().to_string();
            if !remote_url.is_empty() && git_sync::get_remote_url(&dir).is_none() {
                // Initialize with the new remote
                match git_sync::init_with_remote(&dir, &remote_url) {
                    Ok(git_sync::InitResult::PulledRemote) => {
                        log::info!("Pulled config from remote");
                        self.update_git_status_display();
                        // Reload config and trigger callback
                        if let Ok(new_config) = cterm_app::load_config() {
                            if let Some(ref callback) = *self.ivars().on_save.borrow() {
                                callback(new_config);
                            }
                        }
                        return;
                    }
                    Ok(_) => {
                        log::info!("Git remote initialized");
                    }
                    Err(e) => {
                        log::error!("Failed to initialize git remote: {}", e);
                        return;
                    }
                }
            }
        }

        // Perform sync: pull then push
        match git_sync::pull_with_conflict_resolution(&dir) {
            Ok(PullResult::Updated) => {
                log::info!("Pulled updates from remote");
                // Reload config
                if let Ok(new_config) = cterm_app::load_config() {
                    if let Some(ref callback) = *self.ivars().on_save.borrow() {
                        callback(new_config.clone());
                    }
                    *self.ivars().config.borrow_mut() = new_config;
                }
            }
            Ok(PullResult::ConflictsResolved(files)) => {
                log::info!("Pulled with conflicts resolved: {:?}", files);
                if let Ok(new_config) = cterm_app::load_config() {
                    if let Some(ref callback) = *self.ivars().on_save.borrow() {
                        callback(new_config.clone());
                    }
                    *self.ivars().config.borrow_mut() = new_config;
                }
            }
            Ok(PullResult::UpToDate) => {
                log::info!("Already up to date");
            }
            Ok(PullResult::NoRemote) | Ok(PullResult::NotARepo) => {
                log::info!("No remote configured or not a repo");
            }
            Err(e) => {
                log::error!("Sync failed: {}", e);
            }
        }

        // Push any local changes
        if git_sync::is_git_repo(&dir) {
            if let Err(e) = git_sync::commit_and_push(&dir, "Sync configuration") {
                log::error!("Failed to push: {}", e);
            }
        }

        self.update_git_status_display();
    }

    fn update_git_status_display(&self) {
        let status = config_dir()
            .map(|dir| git_sync::get_sync_status(&dir))
            .unwrap_or_default();

        // Update status label
        if let Some(ref label) = *self.ivars().git_status_label.borrow() {
            let status_text = if !status.is_repo {
                "Not initialized"
            } else if status.remote_url.is_none() {
                "No remote configured"
            } else {
                "Configured"
            };
            label.setStringValue(&NSString::from_str(status_text));
        }

        // Update branch label
        if let Some(ref label) = *self.ivars().git_branch_label.borrow() {
            let branch_text = status.branch.clone().unwrap_or_else(|| "-".to_string());
            label.setStringValue(&NSString::from_str(&branch_text));
        }

        // Update last sync label
        if let Some(ref label) = *self.ivars().git_last_sync_label.borrow() {
            let last_sync_text = if let Some(ts) = status.last_commit_time {
                format_timestamp(ts)
            } else {
                "-".to_string()
            };
            label.setStringValue(&NSString::from_str(&last_sync_text));
        }

        // Update changes label
        if let Some(ref label) = *self.ivars().git_changes_label.borrow() {
            let changes_text = if status.has_local_changes {
                "Uncommitted changes"
            } else if status.commits_ahead > 0 && status.commits_behind > 0 {
                "Diverged from remote"
            } else if status.commits_ahead > 0 {
                "Ahead of remote"
            } else if status.commits_behind > 0 {
                "Behind remote"
            } else {
                "Up to date"
            };
            label.setStringValue(&NSString::from_str(changes_text));
        }
    }

    fn create_label_field_row(
        &self,
        mtm: MainThreadMarker,
        label: &str,
        value: &str,
    ) -> (Retained<NSStackView>, Retained<NSTextField>) {
        let row = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Horizontal);
            stack.setSpacing(8.0);
            stack
        };

        let label_view = NSTextField::labelWithString(&NSString::from_str(label), mtm);
        unsafe {
            let _: () = msg_send![&label_view, setAlignment: 2i64]; // NSTextAlignmentRight
        }
        unsafe {
            row.addArrangedSubview(&label_view);
        }

        let field = NSTextField::new(mtm);
        field.setStringValue(&NSString::from_str(value));
        field.setEditable(true);
        field.setBordered(true);
        field.setDrawsBackground(true);
        unsafe {
            let size = NSSize::new(200.0, 22.0);
            let _: () = msg_send![&field, setFrameSize: size];
        }
        unsafe {
            row.addArrangedSubview(&field);
        }

        (row, field)
    }

    fn create_label_popup_row(
        &self,
        mtm: MainThreadMarker,
        label: &str,
        options: &[(&str, &str)],
        selected: &str,
    ) -> (Retained<NSStackView>, Retained<NSPopUpButton>) {
        let row = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Horizontal);
            stack.setSpacing(8.0);
            stack
        };

        let label_view = NSTextField::labelWithString(&NSString::from_str(label), mtm);
        unsafe {
            row.addArrangedSubview(&label_view);
        }

        let popup = unsafe {
            let popup = NSPopUpButton::new(mtm);
            for (id, title) in options {
                popup.addItemWithTitle(&NSString::from_str(title));
                if let Some(item) = popup.lastItem() {
                    item.setRepresentedObject(Some(&NSString::from_str(id)));
                }
            }
            // Select the matching item (match on ID or display name)
            for (i, (id, title)) in options.iter().enumerate() {
                if *id == selected || *title == selected {
                    popup.selectItemAtIndex(i as isize);
                    break;
                }
            }
            popup
        };
        unsafe {
            row.addArrangedSubview(&popup);
        }

        (row, popup)
    }

    fn create_label_slider_row(
        &self,
        mtm: MainThreadMarker,
        label: &str,
        value: f64,
        min: f64,
        max: f64,
    ) -> (Retained<NSStackView>, Retained<NSSlider>) {
        let row = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Horizontal);
            stack.setSpacing(8.0);
            stack
        };

        let label_view = NSTextField::labelWithString(&NSString::from_str(label), mtm);
        unsafe {
            row.addArrangedSubview(&label_view);
        }

        let slider = unsafe {
            let slider = NSSlider::new(mtm);
            slider.setMinValue(min);
            slider.setMaxValue(max);
            slider.setDoubleValue(value);
            let size = NSSize::new(200.0, 22.0);
            let _: () = msg_send![&slider, setFrameSize: size];
            slider
        };
        unsafe {
            row.addArrangedSubview(&slider);
        }

        (row, slider)
    }

    fn create_checkbox(
        &self,
        mtm: MainThreadMarker,
        title: &str,
        checked: bool,
    ) -> Retained<NSButton> {
        let checkbox = unsafe {
            let btn = NSButton::checkboxWithTitle_target_action(
                &NSString::from_str(title),
                None,
                None,
                mtm,
            );
            btn.setState(if checked { 1 } else { 0 });
            btn
        };
        checkbox
    }

    fn collect_and_save(&self) {
        let mut config = self.ivars().config.borrow().clone();

        // Collect General settings
        if let Some(ref field) = *self.ivars().scrollback_field.borrow() {
            let value = field.stringValue().to_string();
            if let Ok(lines) = value.parse::<usize>() {
                config.general.scrollback_lines = lines;
            }
        }
        if let Some(ref checkbox) = *self.ivars().confirm_close_checkbox.borrow() {
            config.general.confirm_close_with_running = checkbox.state() == 1;
        }
        if let Some(ref checkbox) = *self.ivars().copy_on_select_checkbox.borrow() {
            config.general.copy_on_select = checkbox.state() == 1;
        }

        // Collect Appearance settings
        if let Some(ref popup) = *self.ivars().theme_popup.borrow() {
            if let Some(item) = popup.selectedItem() {
                if let Some(obj) = item.representedObject() {
                    let id: &NSString = unsafe { &*(&*obj as *const _ as *const NSString) };
                    config.appearance.theme = id.to_string();
                }
            }
        }
        if let Some(ref field) = *self.ivars().font_field.borrow() {
            config.appearance.font.family = field.stringValue().to_string();
        }
        if let Some(ref field) = *self.ivars().font_size_field.borrow() {
            let value = field.stringValue().to_string();
            if let Ok(size) = value.parse::<f64>() {
                config.appearance.font.size = size;
            }
        }
        if let Some(ref popup) = *self.ivars().cursor_popup.borrow() {
            if let Some(item) = popup.selectedItem() {
                if let Some(obj) = item.representedObject() {
                    let id: &NSString = unsafe { &*(&*obj as *const _ as *const NSString) };
                    config.appearance.cursor_style = match id.to_string().as_str() {
                        "underline" => CursorStyleConfig::Underline,
                        "bar" => CursorStyleConfig::Bar,
                        _ => CursorStyleConfig::Block,
                    };
                }
            }
        }
        if let Some(ref checkbox) = *self.ivars().cursor_blink_checkbox.borrow() {
            config.appearance.cursor_blink = checkbox.state() == 1;
        }
        if let Some(ref slider) = *self.ivars().opacity_slider.borrow() {
            config.appearance.opacity = slider.doubleValue();
        }
        if let Some(ref checkbox) = *self.ivars().bold_bright_checkbox.borrow() {
            config.appearance.bold_is_bright = checkbox.state() == 1;
        }

        // Collect Tabs settings
        if let Some(ref popup) = *self.ivars().show_tab_bar_popup.borrow() {
            if let Some(item) = popup.selectedItem() {
                if let Some(obj) = item.representedObject() {
                    let id: &NSString = unsafe { &*(&*obj as *const _ as *const NSString) };
                    config.tabs.show_tab_bar = match id.to_string().as_str() {
                        "multiple" => TabBarVisibility::Multiple,
                        "never" => TabBarVisibility::Never,
                        _ => TabBarVisibility::Always,
                    };
                }
            }
        }
        if let Some(ref popup) = *self.ivars().tab_position_popup.borrow() {
            if let Some(item) = popup.selectedItem() {
                if let Some(obj) = item.representedObject() {
                    let id: &NSString = unsafe { &*(&*obj as *const _ as *const NSString) };
                    config.tabs.tab_bar_position = match id.to_string().as_str() {
                        "bottom" => TabBarPosition::Bottom,
                        _ => TabBarPosition::Top,
                    };
                }
            }
        }
        if let Some(ref popup) = *self.ivars().new_tab_popup.borrow() {
            if let Some(item) = popup.selectedItem() {
                if let Some(obj) = item.representedObject() {
                    let id: &NSString = unsafe { &*(&*obj as *const _ as *const NSString) };
                    config.tabs.new_tab_position = match id.to_string().as_str() {
                        "after_current" => NewTabPosition::AfterCurrent,
                        _ => NewTabPosition::End,
                    };
                }
            }
        }
        if let Some(ref checkbox) = *self.ivars().show_close_checkbox.borrow() {
            config.tabs.show_close_button = checkbox.state() == 1;
        }

        // Save config to file
        if let Err(e) = save_config(&config) {
            log::error!("Failed to save config: {}", e);
        }

        // Save tool shortcuts
        {
            let entries = self.ivars().tool_entries.borrow();
            let tools: Vec<ToolShortcutEntry> = entries
                .iter()
                .filter_map(|(name_f, cmd_f, args_f)| {
                    let name = name_f.stringValue().to_string();
                    let command = cmd_f.stringValue().to_string();
                    if name.is_empty() || command.is_empty() {
                        return None;
                    }
                    let args_str = args_f.stringValue().to_string();
                    let args: Vec<String> = if args_str.is_empty() {
                        Vec::new()
                    } else {
                        args_str.split_whitespace().map(|s| s.to_string()).collect()
                    };
                    Some(ToolShortcutEntry {
                        name,
                        command,
                        args,
                    })
                })
                .collect();
            if let Err(e) = cterm_app::config::save_tool_shortcuts(&tools) {
                log::error!("Failed to save tool shortcuts: {}", e);
            }
        }

        // Refresh the Tools menu
        {
            let mtm = MainThreadMarker::from(self);
            crate::menu::rebuild_tools_menu(mtm);
        }

        // If git sync is configured, commit and push
        if let Some(dir) = config_dir() {
            if git_sync::is_git_repo(&dir) && git_sync::get_remote_url(&dir).is_some() {
                if let Err(e) = git_sync::commit_and_push(&dir, "Update configuration") {
                    log::error!("Failed to push config: {}", e);
                }
            }
        }

        // Call the on_save callback
        if let Some(ref callback) = *self.ivars().on_save.borrow() {
            callback(config);
        }
    }
}

/// Show the preferences window
pub fn show_preferences(
    mtm: MainThreadMarker,
    config: &Config,
    on_save: impl Fn(Config) + 'static,
) {
    let window = PreferencesWindow::new(mtm, config, on_save);
    window.center();
    window.makeKeyAndOrderFront(None);
}
