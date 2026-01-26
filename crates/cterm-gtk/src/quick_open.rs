//! Quick Open overlay for rapidly searching and opening tab templates
//!
//! Shows a VS Code-style overlay at the top of the window for filtering templates.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cterm_app::config::StickyTabConfig;
use cterm_app::{template_type_indicator, QuickOpenMatcher, TemplateMatch};
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, CssProvider, Entry, EventControllerKey, Label, ListBox, ListBoxRow, Orientation,
    ScrolledWindow,
};

/// Height of the Quick Open overlay
pub const QUICK_OPEN_HEIGHT: i32 = 250;

/// Maximum number of results to display
const MAX_RESULTS: usize = 8;

/// Callback type for template selection
type SelectCallback = Rc<RefCell<Option<Box<dyn Fn(StickyTabConfig)>>>>;

/// Quick Open overlay widget
#[derive(Clone)]
pub struct QuickOpenOverlay {
    /// Main container widget
    container: GtkBox,
    /// Search entry
    search_entry: Entry,
    /// Results list
    results_list: ListBox,
    /// Scrolled window for results
    #[allow(dead_code)]
    results_scroll: ScrolledWindow,
    /// All available templates
    templates: Rc<RefCell<Vec<StickyTabConfig>>>,
    /// Currently filtered templates
    filtered: Rc<RefCell<Vec<TemplateMatch>>>,
    /// Currently selected index
    selected_index: Rc<Cell<usize>>,
    /// Selection callback
    on_select: SelectCallback,
}

impl QuickOpenOverlay {
    /// Create a new Quick Open overlay
    pub fn new() -> Self {
        // Create main container
        let container = GtkBox::new(Orientation::Vertical, 8);
        container.set_height_request(QUICK_OPEN_HEIGHT);
        container.add_css_class("quick-open");
        container.set_visible(false);
        container.set_margin_start(12);
        container.set_margin_end(12);
        container.set_margin_top(12);
        container.set_margin_bottom(12);

        // Apply styling
        let provider = CssProvider::new();
        provider.load_from_data(
            r#"
            .quick-open {
                background-color: rgba(30, 30, 30, 0.95);
                border-radius: 8px;
                padding: 12px;
            }
            .quick-open entry {
                background-color: rgba(50, 50, 50, 0.9);
                color: white;
                border: 1px solid rgba(80, 80, 80, 0.8);
                border-radius: 4px;
                padding: 8px;
                font-size: 14px;
            }
            .quick-open-results {
                background-color: transparent;
            }
            .quick-open-result {
                background-color: transparent;
                padding: 4px 8px;
                border-radius: 4px;
            }
            .quick-open-result:selected,
            .quick-open-result.selected {
                background-color: rgba(0, 102, 204, 1.0);
            }
            .quick-open-result label {
                color: white;
            }
        "#,
        );
        container
            .style_context()
            .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);

        // Create search entry
        let search_entry = Entry::builder()
            .placeholder_text("Search templates...")
            .hexpand(true)
            .build();
        search_entry.add_css_class("quick-open-entry");
        container.append(&search_entry);

        // Create scrolled window for results
        let results_scroll = ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .build();

        // Create results list
        let results_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .build();
        results_list.add_css_class("quick-open-results");
        results_scroll.set_child(Some(&results_list));
        container.append(&results_scroll);

        let templates = Rc::new(RefCell::new(Vec::new()));
        let filtered = Rc::new(RefCell::new(Vec::new()));
        let selected_index = Rc::new(Cell::new(0usize));
        let on_select: SelectCallback = Rc::new(RefCell::new(None));

        let overlay = Self {
            container: container.clone(),
            search_entry: search_entry.clone(),
            results_list: results_list.clone(),
            results_scroll,
            templates,
            filtered,
            selected_index,
            on_select,
        };

        // Connect signals
        overlay.setup_signals();

        overlay
    }

    fn setup_signals(&self) {
        // Search entry text changed
        let overlay_clone = self.clone();
        self.search_entry.connect_changed(move |_| {
            overlay_clone.update_filter();
        });

        // Key press handling for navigation
        let key_controller = EventControllerKey::new();
        let overlay_clone = self.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::Down => {
                overlay_clone.select_next();
                gtk4::glib::Propagation::Stop
            }
            gdk::Key::Up => {
                overlay_clone.select_previous();
                gtk4::glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                overlay_clone.confirm_selection();
                gtk4::glib::Propagation::Stop
            }
            gdk::Key::Escape => {
                overlay_clone.hide();
                gtk4::glib::Propagation::Stop
            }
            gdk::Key::Tab => {
                overlay_clone.select_next();
                gtk4::glib::Propagation::Stop
            }
            _ => gtk4::glib::Propagation::Proceed,
        });
        self.search_entry.add_controller(key_controller);

        // Row activated (double-click or Enter on focused row)
        let overlay_clone = self.clone();
        self.results_list.connect_row_activated(move |_, row| {
            let idx = row.index() as usize;
            overlay_clone.selected_index.set(idx);
            overlay_clone.confirm_selection();
        });
    }

    /// Get the widget to add to the UI
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Set the templates to search through
    pub fn set_templates(&self, templates: Vec<StickyTabConfig>) {
        *self.templates.borrow_mut() = templates;
        self.update_filter();
    }

    /// Show the overlay and focus the search entry
    pub fn show(&self) {
        // Clear search and reset selection
        self.search_entry.set_text("");
        self.selected_index.set(0);

        // Update filter to show all templates
        self.update_filter();

        // Show and focus
        self.container.set_visible(true);
        self.search_entry.grab_focus();
    }

    /// Hide the overlay
    pub fn hide(&self) {
        self.container.set_visible(false);
    }

    /// Check if the overlay is visible
    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        self.container.is_visible()
    }

    /// Set the callback for template selection
    pub fn set_on_select<F>(&self, callback: F)
    where
        F: Fn(StickyTabConfig) + 'static,
    {
        *self.on_select.borrow_mut() = Some(Box::new(callback));
    }

    /// Update the filter based on search text
    fn update_filter(&self) {
        let query = self.search_entry.text().to_string();
        let templates = self.templates.borrow().clone();
        let matcher = QuickOpenMatcher::new(templates);
        let filtered: Vec<TemplateMatch> = matcher
            .filter(&query)
            .into_iter()
            .take(MAX_RESULTS)
            .collect();

        *self.filtered.borrow_mut() = filtered;
        self.selected_index.set(0);
        self.rebuild_results();
    }

    /// Rebuild the results list UI
    fn rebuild_results(&self) {
        // Remove all existing children
        while let Some(child) = self.results_list.first_child() {
            self.results_list.remove(&child);
        }

        let filtered = self.filtered.borrow();
        let selected_idx = self.selected_index.get();

        if filtered.is_empty() {
            // Show "No templates" message
            let row = self.create_result_row("No templates found", "", false);
            self.results_list.append(&row);
            return;
        }

        for (idx, match_result) in filtered.iter().enumerate() {
            let indicator = template_type_indicator(&match_result.template);
            let row =
                self.create_result_row(&match_result.template.name, indicator, idx == selected_idx);
            self.results_list.append(&row);
        }

        // Select the appropriate row
        if let Some(row) = self.results_list.row_at_index(selected_idx as i32) {
            self.results_list.select_row(Some(&row));
        }
    }

    /// Create a result row widget
    fn create_result_row(&self, name: &str, indicator: &str, selected: bool) -> ListBoxRow {
        let row = ListBoxRow::new();
        row.add_css_class("quick-open-result");
        if selected {
            row.add_css_class("selected");
        }

        let content = GtkBox::new(Orientation::Horizontal, 8);
        content.set_margin_start(8);
        content.set_margin_end(8);
        content.set_margin_top(4);
        content.set_margin_bottom(4);

        // Name label
        let name_label = Label::new(Some(name));
        name_label.set_hexpand(true);
        name_label.set_halign(gtk4::Align::Start);
        content.append(&name_label);

        // Indicator label (right side)
        if !indicator.is_empty() {
            let indicator_label = Label::new(Some(indicator));
            indicator_label.set_halign(gtk4::Align::End);
            content.append(&indicator_label);
        }

        row.set_child(Some(&content));
        row
    }

    /// Select the next item
    fn select_next(&self) {
        let filtered = self.filtered.borrow();
        if filtered.is_empty() {
            return;
        }

        let current = self.selected_index.get();
        let new_index = if current + 1 >= filtered.len() {
            0 // Wrap around
        } else {
            current + 1
        };
        drop(filtered);

        self.selected_index.set(new_index);
        self.update_selection_highlight();
    }

    /// Select the previous item
    fn select_previous(&self) {
        let filtered = self.filtered.borrow();
        if filtered.is_empty() {
            return;
        }

        let current = self.selected_index.get();
        let new_index = if current == 0 {
            filtered.len() - 1 // Wrap around
        } else {
            current - 1
        };
        drop(filtered);

        self.selected_index.set(new_index);
        self.update_selection_highlight();
    }

    /// Update the visual selection highlight
    fn update_selection_highlight(&self) {
        let selected_idx = self.selected_index.get();

        // Update CSS classes and ListBox selection
        let mut idx = 0;
        let mut child = self.results_list.first_child();
        while let Some(widget) = child {
            if let Some(row) = widget.downcast_ref::<ListBoxRow>() {
                if idx == selected_idx {
                    row.add_css_class("selected");
                    self.results_list.select_row(Some(row));
                } else {
                    row.remove_css_class("selected");
                }
            }
            idx += 1;
            child = widget.next_sibling();
        }
    }

    /// Confirm the current selection
    fn confirm_selection(&self) {
        let filtered = self.filtered.borrow();
        let selected_idx = self.selected_index.get();

        if let Some(match_result) = filtered.get(selected_idx) {
            let template = match_result.template.clone();
            drop(filtered);

            // Hide first
            self.hide();

            // Call callback
            if let Some(ref callback) = *self.on_select.borrow() {
                callback(template);
            }
        }
    }
}

impl Default for QuickOpenOverlay {
    fn default() -> Self {
        Self::new()
    }
}
