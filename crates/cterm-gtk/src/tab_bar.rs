//! Custom tab bar widget

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, Orientation, Widget};

/// Tab bar widget
#[derive(Clone)]
pub struct TabBar {
    container: GtkBox,
    tabs_box: GtkBox,
    new_tab_button: Button,
    tabs: Rc<RefCell<Vec<TabInfo>>>,
    active_tab: Rc<RefCell<Option<u64>>>,
    on_new_tab: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    on_close_callbacks: Rc<RefCell<HashMap<u64, Box<dyn Fn()>>>>,
    on_click_callbacks: Rc<RefCell<HashMap<u64, Box<dyn Fn()>>>>,
}

struct TabInfo {
    id: u64,
    button: Button,
    label: Label,
    bell_icon: Label,
    close_button: Button,
}

impl TabBar {
    /// Create a new tab bar
    pub fn new() -> Self {
        let container = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(0)
            .build();
        container.add_css_class("tab-bar");

        let tabs_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(2)
            .hexpand(true)
            .build();

        let new_tab_button = Button::builder()
            .label("+")
            .focusable(false)
            .build();
        new_tab_button.add_css_class("new-tab-button");

        container.append(&tabs_box);
        container.append(&new_tab_button);

        let tab_bar = Self {
            container,
            tabs_box,
            new_tab_button: new_tab_button.clone(),
            tabs: Rc::new(RefCell::new(Vec::new())),
            active_tab: Rc::new(RefCell::new(None)),
            on_new_tab: Rc::new(RefCell::new(None)),
            on_close_callbacks: Rc::new(RefCell::new(HashMap::new())),
            on_click_callbacks: Rc::new(RefCell::new(HashMap::new())),
        };

        // Set up new tab button click
        let on_new_tab = Rc::clone(&tab_bar.on_new_tab);
        new_tab_button.connect_clicked(move |_| {
            if let Some(ref callback) = *on_new_tab.borrow() {
                callback();
            }
        });

        tab_bar
    }

    /// Get the widget
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Add a new tab
    pub fn add_tab(&self, id: u64, title: &str) {
        let tab_box = GtkBox::new(Orientation::Horizontal, 4);

        // Bell icon (hidden by default)
        let bell_icon = Label::new(Some("🔔"));
        bell_icon.set_visible(false);
        bell_icon.add_css_class("tab-bell-icon");

        let label = Label::new(Some(title));

        let close_button = Button::builder()
            .label("×")
            .focusable(false)
            .build();
        close_button.add_css_class("tab-close-button");

        tab_box.append(&bell_icon);
        tab_box.append(&label);
        tab_box.append(&close_button);

        let button = Button::builder()
            .child(&tab_box)
            .focusable(false)
            .build();

        // Set up close button
        let close_callbacks = Rc::clone(&self.on_close_callbacks);
        let tab_id = id;
        close_button.connect_clicked(move |_| {
            if let Some(callback) = close_callbacks.borrow().get(&tab_id) {
                callback();
            }
        });

        // Set up tab button click
        let click_callbacks = Rc::clone(&self.on_click_callbacks);
        let active_tab = Rc::clone(&self.active_tab);
        let tabs = Rc::clone(&self.tabs);
        button.connect_clicked(move |btn| {
            // Update active state visually
            for tab in tabs.borrow().iter() {
                tab.button.remove_css_class("active");
            }
            btn.add_css_class("active");
            *active_tab.borrow_mut() = Some(tab_id);

            if let Some(callback) = click_callbacks.borrow().get(&tab_id) {
                callback();
            }
        });

        self.tabs_box.append(&button);

        self.tabs.borrow_mut().push(TabInfo {
            id,
            button,
            label,
            bell_icon,
            close_button,
        });

        // Set as active if first tab
        if self.tabs.borrow().len() == 1 {
            self.set_active(id);
        }
    }

    /// Remove a tab
    pub fn remove_tab(&self, id: u64) {
        let mut tabs = self.tabs.borrow_mut();
        if let Some(idx) = tabs.iter().position(|t| t.id == id) {
            let tab = tabs.remove(idx);
            self.tabs_box.remove(&tab.button);
        }

        // Remove callbacks
        self.on_close_callbacks.borrow_mut().remove(&id);
        self.on_click_callbacks.borrow_mut().remove(&id);
    }

    /// Set the active tab
    pub fn set_active(&self, id: u64) {
        *self.active_tab.borrow_mut() = Some(id);

        for tab in self.tabs.borrow().iter() {
            if tab.id == id {
                tab.button.add_css_class("active");
            } else {
                tab.button.remove_css_class("active");
            }
        }
    }

    /// Update tab title
    pub fn set_title(&self, id: u64, title: &str) {
        for tab in self.tabs.borrow().iter() {
            if tab.id == id {
                tab.label.set_text(title);
                break;
            }
        }
    }

    /// Set tab color
    pub fn set_color(&self, id: u64, color: Option<&str>) {
        for tab in self.tabs.borrow().iter() {
            if tab.id == id {
                if let Some(color) = color {
                    // Apply inline style for color
                    let css = format!("border-left: 3px solid {};", color);
                    // Note: In a real implementation, we'd use a CSS provider
                    // For now, we just add a class
                    tab.button.add_css_class("colored-tab");
                } else {
                    tab.button.remove_css_class("colored-tab");
                }
                break;
            }
        }
    }

    /// Mark tab as having unread content
    pub fn set_unread(&self, id: u64, unread: bool) {
        for tab in self.tabs.borrow().iter() {
            if tab.id == id {
                if unread {
                    tab.button.add_css_class("has-unread");
                } else {
                    tab.button.remove_css_class("has-unread");
                }
                break;
            }
        }
    }

    /// Set callback for new tab button
    pub fn set_on_new_tab<F: Fn() + 'static>(&self, callback: F) {
        *self.on_new_tab.borrow_mut() = Some(Box::new(callback));
    }

    /// Set callback for tab close
    pub fn set_on_close<F: Fn() + 'static>(&self, id: u64, callback: F) {
        self.on_close_callbacks
            .borrow_mut()
            .insert(id, Box::new(callback));
    }

    /// Set callback for tab click
    pub fn set_on_click<F: Fn() + 'static>(&self, id: u64, callback: F) {
        self.on_click_callbacks
            .borrow_mut()
            .insert(id, Box::new(callback));
    }

    /// Get number of tabs
    pub fn tab_count(&self) -> usize {
        self.tabs.borrow().len()
    }

    /// Set bell indicator visibility for a tab
    pub fn set_bell(&self, id: u64, visible: bool) {
        for tab in self.tabs.borrow().iter() {
            if tab.id == id {
                tab.bell_icon.set_visible(visible);
                break;
            }
        }
    }

    /// Clear bell indicator for a tab (convenience wrapper)
    pub fn clear_bell(&self, id: u64) {
        self.set_bell(id, false);
    }
}

impl Default for TabBar {
    fn default() -> Self {
        Self::new()
    }
}
