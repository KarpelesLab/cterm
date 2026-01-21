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

use cterm_app::config::{
    save_config, Config, CursorStyleConfig, NewTabPosition, TabBarPosition, TabBarVisibility,
};

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
        // Create main vertical stack
        let main_stack = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Vertical);
            stack.setSpacing(12.0);
            stack.setEdgeInsets(objc2_foundation::NSEdgeInsets {
                top: 12.0,
                left: 12.0,
                bottom: 12.0,
                right: 12.0,
            });
            stack
        };

        // Create tab view
        let tab_view = NSTabView::new(mtm);

        // Add tabs
        let general_tab = self.create_general_tab(mtm, config);
        tab_view.addTabViewItem(&general_tab);

        let appearance_tab = self.create_appearance_tab(mtm, config);
        tab_view.addTabViewItem(&appearance_tab);

        let tabs_tab = self.create_tabs_tab(mtm, config);
        tab_view.addTabViewItem(&tabs_tab);

        unsafe {
            main_stack.addArrangedSubview(&tab_view);
        }

        // Create button row
        let button_stack = unsafe {
            let stack = NSStackView::new(mtm);
            stack.setOrientation(objc2_app_kit::NSUserInterfaceLayoutOrientation::Horizontal);
            stack.setSpacing(8.0);
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
            main_stack.addArrangedSubview(&button_stack);
        }

        self.setContentView(Some(&main_stack));
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
            // Select the matching item
            for (i, (id, _)) in options.iter().enumerate() {
                if *id == selected {
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

        // Save to file
        if let Err(e) = save_config(&config) {
            log::error!("Failed to save config: {}", e);
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
