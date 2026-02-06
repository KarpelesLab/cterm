//! Custom tab bar widget

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::gio::{Menu, SimpleAction, SimpleActionGroup};
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, GestureClick, Label, Orientation, PopoverMenu};

/// Callback type for tab bar events
type TabCallback = Rc<RefCell<Option<Box<dyn Fn()>>>>;
/// Callback map type for per-tab callbacks
type TabCallbackMap = Rc<RefCell<HashMap<u64, Box<dyn Fn()>>>>;
/// Callback type for tab-specific events with tab ID
type TabIdCallback = Rc<RefCell<Option<Box<dyn Fn(u64)>>>>;

/// Tab bar widget
#[derive(Clone)]
pub struct TabBar {
    container: GtkBox,
    tabs_box: GtkBox,
    #[allow(dead_code)] // Kept to prevent button from being dropped
    new_tab_button: Button,
    tabs: Rc<RefCell<Vec<TabInfo>>>,
    active_tab: Rc<RefCell<Option<u64>>>,
    on_new_tab: TabCallback,
    on_close_callbacks: TabCallbackMap,
    on_click_callbacks: TabCallbackMap,
    on_rename: TabIdCallback,
    on_set_color: TabIdCallback,
    /// Current tab ID for context menu actions
    context_menu_tab_id: Rc<RefCell<Option<u64>>>,
}

struct TabInfo {
    id: u64,
    button: Button,
    label: Label,
    bell_icon: Label,
    #[allow(dead_code)] // Kept to prevent button from being dropped
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

        let new_tab_button = Button::builder().label("+").focusable(false).build();
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
            on_rename: Rc::new(RefCell::new(None)),
            on_set_color: Rc::new(RefCell::new(None)),
            context_menu_tab_id: Rc::new(RefCell::new(None)),
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

        let close_button = Button::builder().label("×").focusable(false).build();
        close_button.add_css_class("tab-close-button");

        tab_box.append(&bell_icon);
        tab_box.append(&label);
        tab_box.append(&close_button);

        let button = Button::builder().child(&tab_box).focusable(false).build();

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

        // Set up right-click context menu
        let context_menu_tab_id = Rc::clone(&self.context_menu_tab_id);
        let on_rename = Rc::clone(&self.on_rename);
        let on_set_color = Rc::clone(&self.on_set_color);

        // Create action group for this tab's context menu
        let action_group = SimpleActionGroup::new();

        let rename_action = SimpleAction::new("rename", None);
        let context_id_rename = Rc::clone(&context_menu_tab_id);
        let on_rename_clone = Rc::clone(&on_rename);
        rename_action.connect_activate(move |_, _| {
            if let Some(id) = *context_id_rename.borrow() {
                if let Some(ref callback) = *on_rename_clone.borrow() {
                    callback(id);
                }
            }
        });
        action_group.add_action(&rename_action);

        let color_action = SimpleAction::new("set-color", None);
        let context_id_color = Rc::clone(&context_menu_tab_id);
        let on_set_color_clone = Rc::clone(&on_set_color);
        color_action.connect_activate(move |_, _| {
            if let Some(id) = *context_id_color.borrow() {
                if let Some(ref callback) = *on_set_color_clone.borrow() {
                    callback(id);
                }
            }
        });
        action_group.add_action(&color_action);

        button.insert_action_group("tab", Some(&action_group));

        // Create context menu
        let menu = Menu::new();
        menu.append(Some("Rename Tab..."), Some("tab.rename"));
        menu.append(Some("Set Tab Color..."), Some("tab.set-color"));

        let popover = PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&button);
        popover.set_has_arrow(false);

        // Right-click gesture
        let gesture = GestureClick::new();
        gesture.set_button(3); // Right mouse button
        let popover_clone = popover.clone();
        let context_menu_tab_id_gesture = Rc::clone(&context_menu_tab_id);
        gesture.connect_pressed(move |gesture, _, x, y| {
            *context_menu_tab_id_gesture.borrow_mut() = Some(tab_id);
            // Position the popover at click location
            popover_clone
                .set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover_clone.popup();
            gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
        button.add_controller(gesture);

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
                    // Apply inline style using CSS provider
                    let css = format!(
                        "button.colored-tab-{} {{ background-color: {}; }}",
                        id, color
                    );
                    let provider = gtk4::CssProvider::new();
                    provider.load_from_data(&css);

                    // Remove old colored-tab class if any
                    let classes: Vec<_> = tab
                        .button
                        .css_classes()
                        .iter()
                        .filter(|c| c.starts_with("colored-tab"))
                        .map(|c| c.to_string())
                        .collect();
                    for class in classes {
                        tab.button.remove_css_class(&class);
                    }

                    // Add new class and style
                    let class_name = format!("colored-tab-{}", id);
                    tab.button.add_css_class(&class_name);

                    if let Some(display) = gtk4::gdk::Display::default() {
                        gtk4::style_context_add_provider_for_display(
                            &display,
                            &provider,
                            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                        );
                    }
                } else {
                    // Remove any colored-tab class
                    let classes: Vec<_> = tab
                        .button
                        .css_classes()
                        .iter()
                        .filter(|c| c.starts_with("colored-tab"))
                        .map(|c| c.to_string())
                        .collect();
                    for class in classes {
                        tab.button.remove_css_class(&class);
                    }
                }
                break;
            }
        }
    }

    /// Mark tab as having unread content
    #[allow(dead_code)]
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

    /// Set callback for tab rename (from context menu)
    #[allow(dead_code)]
    pub fn set_on_rename<F: Fn(u64) + 'static>(&self, callback: F) {
        *self.on_rename.borrow_mut() = Some(Box::new(callback));
    }

    /// Set callback for tab set color (from context menu)
    #[allow(dead_code)]
    pub fn set_on_set_color<F: Fn(u64) + 'static>(&self, callback: F) {
        *self.on_set_color.borrow_mut() = Some(Box::new(callback));
    }

    /// Get number of tabs
    #[allow(dead_code)]
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

    /// Check if a specific tab has a bell indicator active
    pub fn has_bell(&self, id: u64) -> bool {
        self.tabs
            .borrow()
            .iter()
            .any(|tab| tab.id == id && tab.bell_icon.is_visible())
    }

    /// Update tab bar visibility based on tab count
    /// Hide when there's only one tab, show when there are multiple
    pub fn update_visibility(&self) {
        let tab_count = self.tabs.borrow().len();
        self.container.set_visible(tab_count > 1);
    }

    /// Check if any tab has a bell indicator active
    #[allow(dead_code)]
    pub fn has_any_bell(&self) -> bool {
        self.tabs
            .borrow()
            .iter()
            .any(|tab| tab.bell_icon.is_visible())
    }
}

impl Default for TabBar {
    fn default() -> Self {
        Self::new()
    }
}
