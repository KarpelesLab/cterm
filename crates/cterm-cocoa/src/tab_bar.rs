//! Tab bar implementation for macOS
//!
//! Native macOS tab bar using NSStackView or NSSegmentedControl.

use std::cell::RefCell;
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::{class, define_class, msg_send, AllocAnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSButton, NSStackView, NSStackViewGravity, NSUserInterfaceLayoutOrientation};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use cterm_ui::traits::TabInfo;

/// Tab entry with button and metadata
struct TabEntry {
    id: u64,
    title: String,
    button: Retained<NSButton>,
    active: bool,
    has_bell: bool,
}

/// Tab bar state
pub struct TabBarIvars {
    tabs: RefCell<Vec<TabEntry>>,
    tab_buttons: RefCell<HashMap<u64, Retained<NSButton>>>,
    active_tab: RefCell<Option<u64>>,
    on_tab_click: RefCell<Option<Box<dyn Fn(u64)>>>,
    on_tab_close: RefCell<Option<Box<dyn Fn(u64)>>>,
    on_new_tab: RefCell<Option<Box<dyn Fn()>>>,
}

define_class!(
    #[unsafe(super(NSStackView))]
    #[thread_kind = MainThreadOnly]
    #[name = "TabBar"]
    #[ivars = TabBarIvars]
    pub struct TabBar;

    unsafe impl NSObjectProtocol for TabBar {}
);

impl TabBar {
    /// Create a new tab bar
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(800.0, 28.0));

        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(TabBarIvars {
            tabs: RefCell::new(Vec::new()),
            tab_buttons: RefCell::new(HashMap::new()),
            active_tab: RefCell::new(None),
            on_tab_click: RefCell::new(None),
            on_tab_close: RefCell::new(None),
            on_new_tab: RefCell::new(None),
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Configure stack view
        this.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        this.setSpacing(4.0);
        this.setEdgeInsets(objc2_foundation::NSEdgeInsets {
            top: 4.0,
            left: 8.0,
            bottom: 4.0,
            right: 8.0,
        });

        // Set distribution
        unsafe {
            let _: () = msg_send![&*this, setDistribution: 0i64]; // NSStackViewDistributionFill = 0
        }

        // Enable layer backing and set background color for visibility
        unsafe {
            let _: () = msg_send![&*this, setWantsLayer: true];
            let layer: *mut objc2::runtime::AnyObject = msg_send![&*this, layer];
            if !layer.is_null() {
                // Light gray background
                let cg_color: *mut objc2::runtime::AnyObject = msg_send![
                    class!(NSColor),
                    colorWithRed: 0.9f64,
                    green: 0.9f64,
                    blue: 0.9f64,
                    alpha: 1.0f64
                ];
                let cg_color_ref: *mut objc2::runtime::AnyObject = msg_send![cg_color, CGColor];
                let _: () = msg_send![layer, setBackgroundColor: cg_color_ref];
            }
        }

        // Add "new tab" button
        let new_tab_button = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str("+"), None, None, mtm)
        };
        unsafe {
            let _: () = msg_send![&*new_tab_button, setBezelStyle: 1i64]; // NSBezelStyleRounded
        }
        this.addView_inGravity(&new_tab_button, NSStackViewGravity::Trailing);

        log::debug!("TabBar created");
        this
    }

    /// Add a new tab
    pub fn add_tab(&self, id: u64, title: &str) {
        let mtm = MainThreadMarker::from(self);

        // Create tab button with proper styling
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, mtm)
        };

        // Style the button
        unsafe {
            let _: () = msg_send![&*button, setBezelStyle: 1i64]; // NSBezelStyleRounded
            let _: () = msg_send![&*button, setButtonType: 0i64]; // NSButtonTypeMomentaryLight
        }

        // Store in our list
        self.ivars().tabs.borrow_mut().push(TabEntry {
            id,
            title: title.to_string(),
            button: button.clone(),
            active: false,
            has_bell: false,
        });

        self.ivars()
            .tab_buttons
            .borrow_mut()
            .insert(id, button.clone());

        // Add to stack view (before the + button)
        let count = self.views().len();
        log::info!("Adding tab {} - views before: {}", id, count);

        // Simply add to leading gravity
        self.addView_inGravity(&button, NSStackViewGravity::Leading);

        let count_after = self.views().len();
        log::info!(
            "Tab {} added - views after: {}, hidden: {}",
            id,
            count_after,
            self.isHidden()
        );
    }

    /// Remove a tab
    pub fn remove_tab(&self, id: u64) {
        let mut tabs = self.ivars().tabs.borrow_mut();
        if let Some(pos) = tabs.iter().position(|t| t.id == id) {
            let entry = tabs.remove(pos);
            self.removeView(&entry.button);
        }
        self.ivars().tab_buttons.borrow_mut().remove(&id);
    }

    /// Set active tab
    pub fn set_active(&self, id: u64) {
        *self.ivars().active_tab.borrow_mut() = Some(id);

        // Update button states
        for tab in self.ivars().tabs.borrow_mut().iter_mut() {
            tab.active = tab.id == id;
            // Update button appearance based on active state
            // NSButton doesn't have a built-in "selected" state,
            // so we'd need to use bezel style or color changes
        }
    }

    /// Set tab title
    pub fn set_title(&self, id: u64, title: &str) {
        if let Some(button) = self.ivars().tab_buttons.borrow().get(&id) {
            button.setTitle(&NSString::from_str(title));
        }

        for tab in self.ivars().tabs.borrow_mut().iter_mut() {
            if tab.id == id {
                tab.title = title.to_string();
                break;
            }
        }
    }

    /// Set bell indicator
    pub fn set_bell(&self, id: u64, has_bell: bool) {
        for tab in self.ivars().tabs.borrow_mut().iter_mut() {
            if tab.id == id {
                tab.has_bell = has_bell;
                // Update button title to show bell
                let title = if has_bell {
                    format!("🔔 {}", tab.title)
                } else {
                    tab.title.clone()
                };
                tab.button.setTitle(&NSString::from_str(&title));
                break;
            }
        }
    }

    /// Clear bell indicator
    pub fn clear_bell(&self, id: u64) {
        self.set_bell(id, false);
    }

    /// Get active tab ID
    pub fn active_tab(&self) -> Option<u64> {
        *self.ivars().active_tab.borrow()
    }

    /// Get all tab IDs
    pub fn tab_ids(&self) -> Vec<u64> {
        self.ivars().tabs.borrow().iter().map(|t| t.id).collect()
    }

    /// Set callback for tab click
    pub fn set_on_click<F: Fn(u64) + 'static>(&self, callback: F) {
        *self.ivars().on_tab_click.borrow_mut() = Some(Box::new(callback));
    }

    /// Set callback for tab close
    pub fn set_on_close<F: Fn(u64) + 'static>(&self, callback: F) {
        *self.ivars().on_tab_close.borrow_mut() = Some(Box::new(callback));
    }

    /// Set callback for new tab
    pub fn set_on_new_tab<F: Fn() + 'static>(&self, callback: F) {
        *self.ivars().on_new_tab.borrow_mut() = Some(Box::new(callback));
    }

    /// Update visibility based on tab count
    pub fn update_visibility(&self) {
        let count = self.ivars().tabs.borrow().len();
        // Only show tab bar if more than one tab
        self.setHidden(count <= 1);
    }
}

impl cterm_ui::traits::TabBar for TabBar {
    fn add_tab(&mut self, info: TabInfo) {
        TabBar::add_tab(self, info.id, &info.title);
        if info.active {
            TabBar::set_active(self, info.id);
        }
    }

    fn remove_tab(&mut self, id: u64) {
        TabBar::remove_tab(self, id);
    }

    fn update_tab(&mut self, info: TabInfo) {
        TabBar::set_title(self, info.id, &info.title);
        if info.has_unread {
            TabBar::set_bell(self, info.id, true);
        }
        if info.active {
            TabBar::set_active(self, info.id);
        }
    }

    fn set_active(&mut self, id: u64) {
        TabBar::set_active(self, id);
    }

    fn active_tab(&self) -> Option<u64> {
        TabBar::active_tab(self)
    }

    fn tab_ids(&self) -> Vec<u64> {
        TabBar::tab_ids(self)
    }

    fn reorder(&mut self, _from: usize, _to: usize) {
        // TODO: Implement tab reordering
    }
}
