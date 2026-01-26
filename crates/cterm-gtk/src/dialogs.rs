//! Dialog windows for menu actions

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, ComboBoxText, Dialog, Entry, Grid, Label, Orientation,
    ResponseType, ScrolledWindow, SpinButton, Switch, Window,
};

use cterm_app::config::{
    config_dir, Config, CursorStyleConfig, NewTabPosition, TabBarPosition, TabBarVisibility,
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

/// Show the "Set Title" dialog
pub fn show_set_title_dialog<F>(parent: &impl IsA<Window>, current_title: &str, callback: F)
where
    F: Fn(String) + 'static,
{
    let dialog = Dialog::builder()
        .title("Set Tab Title")
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("OK", ResponseType::Ok);

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let label = Label::new(Some("Tab title:"));
    label.set_halign(Align::Start);
    content.append(&label);

    let entry = Entry::new();
    entry.set_text(current_title);
    entry.set_hexpand(true);
    content.append(&entry);

    let entry_clone = entry.clone();
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Ok {
            let title = entry_clone.text().to_string();
            callback(title);
        }
        dialog.close();
    });

    dialog.present();
}

/// Show the "Set Color" dialog
pub fn show_set_color_dialog<F>(parent: &impl IsA<Window>, callback: F)
where
    F: Fn(Option<String>) + 'static,
{
    let dialog = Dialog::builder()
        .title("Set Tab Color")
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("Clear", ResponseType::Reject);
    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("OK", ResponseType::Ok);

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let label = Label::new(Some("Select a color for this tab:"));
    label.set_halign(Align::Start);
    content.append(&label);

    // Color picker row with ColorButton and hex entry
    let picker_box = GtkBox::new(Orientation::Horizontal, 12);

    let color_button = gtk4::ColorButton::new();
    color_button.set_tooltip_text(Some("Choose custom color"));
    picker_box.append(&color_button);

    let hex_label = Label::new(Some("Hex:"));
    picker_box.append(&hex_label);

    let hex_entry = Entry::new();
    hex_entry.set_placeholder_text(Some("#RRGGBB"));
    hex_entry.set_max_length(7);
    hex_entry.set_width_request(100);
    picker_box.append(&hex_entry);

    content.append(&picker_box);

    // Preset color buttons
    let presets_label = Label::new(Some("Presets:"));
    presets_label.set_halign(Align::Start);
    presets_label.set_margin_top(8);
    content.append(&presets_label);

    let colors_box = GtkBox::new(Orientation::Horizontal, 8);
    let colors = [
        ("#e74c3c", "Red"),
        ("#e67e22", "Orange"),
        ("#f1c40f", "Yellow"),
        ("#2ecc71", "Green"),
        ("#3498db", "Blue"),
        ("#9b59b6", "Purple"),
        ("#1abc9c", "Teal"),
        ("#95a5a6", "Gray"),
    ];

    let selected_color: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // Sync color button changes to hex entry and selected color
    let hex_entry_clone = hex_entry.clone();
    let selected_for_button = Rc::clone(&selected_color);
    color_button.connect_rgba_notify(move |btn| {
        let rgba = btn.rgba();
        let hex = format!(
            "#{:02X}{:02X}{:02X}",
            (rgba.red() * 255.0) as u8,
            (rgba.green() * 255.0) as u8,
            (rgba.blue() * 255.0) as u8
        );
        hex_entry_clone.set_text(&hex);
        *selected_for_button.borrow_mut() = Some(hex);
    });

    // Sync hex entry changes to color button and selected color
    let color_button_clone = color_button.clone();
    let selected_for_entry = Rc::clone(&selected_color);
    hex_entry.connect_changed(move |entry| {
        let text = entry.text();
        if let Some(rgba) = parse_hex_to_rgba(&text) {
            color_button_clone.set_rgba(&rgba);
            *selected_for_entry.borrow_mut() = Some(text.to_string());
        }
    });

    for (color, name) in colors {
        let btn = Button::new();
        btn.set_tooltip_text(Some(name));
        btn.set_size_request(32, 32);
        // Apply color via CSS
        let css = format!(
            "button {{ background: {}; min-width: 32px; min-height: 32px; }}",
            color
        );
        let provider = gtk4::CssProvider::new();
        provider.load_from_data(&css);
        btn.style_context()
            .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);

        let color_str = color.to_string();
        let selected = Rc::clone(&selected_color);
        let hex_entry = hex_entry.clone();
        let color_button = color_button.clone();
        btn.connect_clicked(move |_| {
            hex_entry.set_text(&color_str);
            if let Some(rgba) = parse_hex_to_rgba(&color_str) {
                color_button.set_rgba(&rgba);
            }
            *selected.borrow_mut() = Some(color_str.clone());
        });

        colors_box.append(&btn);
    }
    content.append(&colors_box);

    let selected_for_response = Rc::clone(&selected_color);
    dialog.connect_response(move |dialog, response| {
        match response {
            ResponseType::Ok => {
                let color = selected_for_response.borrow().clone();
                callback(color);
            }
            ResponseType::Reject => {
                callback(None);
            }
            _ => {}
        }
        dialog.close();
    });

    dialog.present();
}

/// Parse hex color string to RGBA
fn parse_hex_to_rgba(hex: &str) -> Option<gtk4::gdk::RGBA> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;

    Some(gtk4::gdk::RGBA::new(r, g, b, 1.0))
}

/// Show the "Find" dialog
pub fn show_find_dialog<F>(parent: &impl IsA<Window>, callback: F)
where
    F: Fn(String, bool, bool) + 'static,
{
    let dialog = Dialog::builder()
        .title("Find in Terminal")
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("Close", ResponseType::Close);
    dialog.add_button("Find Next", ResponseType::Ok);

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let grid = Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(12);

    let search_label = Label::new(Some("Search:"));
    search_label.set_halign(Align::End);
    grid.attach(&search_label, 0, 0, 1, 1);

    let search_entry = Entry::new();
    search_entry.set_hexpand(true);
    grid.attach(&search_entry, 1, 0, 2, 1);

    let case_check = gtk4::CheckButton::with_label("Case sensitive");
    grid.attach(&case_check, 1, 1, 1, 1);

    let regex_check = gtk4::CheckButton::with_label("Regular expression");
    grid.attach(&regex_check, 2, 1, 1, 1);

    content.append(&grid);

    let entry_clone = search_entry.clone();
    let case_clone = case_check.clone();
    let regex_clone = regex_check.clone();

    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Ok {
            let text = entry_clone.text().to_string();
            let case_sensitive = case_clone.is_active();
            let regex = regex_clone.is_active();
            callback(text, case_sensitive, regex);
        } else {
            dialog.close();
        }
    });

    dialog.present();
}

/// Show the About dialog
pub fn show_about_dialog(parent: &impl IsA<Window>) {
    let about = gtk4::AboutDialog::builder()
        .transient_for(parent)
        .modal(true)
        .program_name("cterm")
        .version(env!("CARGO_PKG_VERSION"))
        .comments("A modern terminal emulator built with Rust and GTK4")
        .website("https://github.com/KarpelesLab/cterm")
        .website_label("GitHub Repository")
        .license_type(gtk4::License::MitX11)
        .authors(vec!["KarpelesLab"])
        .build();

    about.present();
}

/// Widgets for collecting preference values
struct PreferencesWidgets {
    // General
    scrollback_spin: SpinButton,
    confirm_switch: Switch,
    copy_select_switch: Switch,
    // Appearance
    theme_combo: ComboBoxText,
    font_entry: Entry,
    size_spin: SpinButton,
    cursor_combo: ComboBoxText,
    blink_switch: Switch,
    opacity_scale: gtk4::Scale,
    bold_switch: Switch,
    // Tabs
    show_combo: ComboBoxText,
    position_combo: ComboBoxText,
    new_combo: ComboBoxText,
    close_switch: Switch,
    // Shortcuts
    shortcut_entries: Vec<(String, Entry)>,
    // Git Sync
    git_remote_entry: Entry,
    git_status_label: Label,
    git_branch_label: Label,
    git_last_sync_label: Label,
    git_changes_label: Label,
    on_save_callback: Rc<RefCell<Option<Box<dyn Fn(Config)>>>>,
    base_config: Rc<RefCell<Config>>,
}

impl PreferencesWidgets {
    /// Perform sync now - pull then push
    fn sync_now(&self) {
        let Some(dir) = config_dir() else {
            log::error!("No config directory found");
            return;
        };

        // First, check if we need to initialize with remote
        let remote_url = self.git_remote_entry.text().to_string();
        if !remote_url.is_empty() && git_sync::get_remote_url(&dir).is_none() {
            // Initialize with the new remote
            match git_sync::init_with_remote(&dir, &remote_url) {
                Ok(git_sync::InitResult::PulledRemote) => {
                    log::info!("Pulled config from remote");
                    self.update_status_display();
                    // Reload config and trigger callback
                    if let Ok(new_config) = cterm_app::load_config() {
                        if let Some(ref callback) = *self.on_save_callback.borrow() {
                            callback(new_config.clone());
                        }
                        *self.base_config.borrow_mut() = new_config;
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

        // Perform sync: pull then push
        match git_sync::pull_with_conflict_resolution(&dir) {
            Ok(PullResult::Updated) => {
                log::info!("Pulled updates from remote");
                // Reload config
                if let Ok(new_config) = cterm_app::load_config() {
                    if let Some(ref callback) = *self.on_save_callback.borrow() {
                        callback(new_config.clone());
                    }
                    *self.base_config.borrow_mut() = new_config;
                }
            }
            Ok(PullResult::ConflictsResolved(files)) => {
                log::info!("Pulled with conflicts resolved: {:?}", files);
                if let Ok(new_config) = cterm_app::load_config() {
                    if let Some(ref callback) = *self.on_save_callback.borrow() {
                        callback(new_config.clone());
                    }
                    *self.base_config.borrow_mut() = new_config;
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

        self.update_status_display();
    }

    /// Update the status display labels
    fn update_status_display(&self) {
        let status = config_dir()
            .map(|dir| git_sync::get_sync_status(&dir))
            .unwrap_or_default();

        // Update status label
        let status_text = if !status.is_repo {
            "Not initialized"
        } else if status.remote_url.is_none() {
            "No remote configured"
        } else {
            "Configured"
        };
        self.git_status_label.set_text(status_text);

        // Update branch label
        let branch_text = status.branch.clone().unwrap_or_else(|| "-".to_string());
        self.git_branch_label.set_text(&branch_text);

        // Update last sync label
        let last_sync_text = if let Some(ts) = status.last_commit_time {
            format_timestamp(ts)
        } else {
            "-".to_string()
        };
        self.git_last_sync_label.set_text(&last_sync_text);

        // Update changes label
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
        self.git_changes_label.set_text(changes_text);
    }

    fn collect_config(&self, base_config: &Config) -> Config {
        let mut config = base_config.clone();

        // General
        config.general.scrollback_lines = self.scrollback_spin.value() as usize;
        config.general.confirm_close_with_running = self.confirm_switch.is_active();
        config.general.copy_on_select = self.copy_select_switch.is_active();

        // Appearance
        if let Some(theme_id) = self.theme_combo.active_id() {
            config.appearance.theme = theme_id.to_string();
        }
        config.appearance.font.family = self.font_entry.text().to_string();
        config.appearance.font.size = self.size_spin.value();
        config.appearance.cursor_style = match self.cursor_combo.active_id().as_deref() {
            Some("underline") => CursorStyleConfig::Underline,
            Some("bar") => CursorStyleConfig::Bar,
            _ => CursorStyleConfig::Block,
        };
        config.appearance.cursor_blink = self.blink_switch.is_active();
        config.appearance.opacity = self.opacity_scale.value();
        config.appearance.bold_is_bright = self.bold_switch.is_active();

        // Tabs
        config.tabs.show_tab_bar = match self.show_combo.active_id().as_deref() {
            Some("multiple") => TabBarVisibility::Multiple,
            Some("never") => TabBarVisibility::Never,
            _ => TabBarVisibility::Always,
        };
        config.tabs.tab_bar_position = match self.position_combo.active_id().as_deref() {
            Some("bottom") => TabBarPosition::Bottom,
            _ => TabBarPosition::Top,
        };
        config.tabs.new_tab_position = match self.new_combo.active_id().as_deref() {
            Some("after_current") => NewTabPosition::AfterCurrent,
            _ => NewTabPosition::End,
        };
        config.tabs.show_close_button = self.close_switch.is_active();

        // Shortcuts
        for (name, entry) in &self.shortcut_entries {
            let value = entry.text().to_string();
            match name.as_str() {
                "new_tab" => config.shortcuts.new_tab = value,
                "close_tab" => config.shortcuts.close_tab = value,
                "next_tab" => config.shortcuts.next_tab = value,
                "prev_tab" => config.shortcuts.prev_tab = value,
                "new_window" => config.shortcuts.new_window = value,
                "close_window" => config.shortcuts.close_window = value,
                "copy" => config.shortcuts.copy = value,
                "paste" => config.shortcuts.paste = value,
                "select_all" => config.shortcuts.select_all = value,
                "zoom_in" => config.shortcuts.zoom_in = value,
                "zoom_out" => config.shortcuts.zoom_out = value,
                "zoom_reset" => config.shortcuts.zoom_reset = value,
                "find" => config.shortcuts.find = value,
                "reset" => config.shortcuts.reset = value,
                _ => {}
            }
        }

        config
    }
}

/// Show the Preferences dialog
pub fn show_preferences_dialog(
    parent: &impl IsA<Window>,
    config: &Config,
    on_save: impl Fn(Config) + 'static,
) {
    let dialog = Dialog::builder()
        .title("Preferences")
        .transient_for(parent)
        .modal(true)
        .default_width(500)
        .default_height(400)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("Apply", ResponseType::Apply);
    dialog.add_button("OK", ResponseType::Ok);

    let content = dialog.content_area();
    content.set_spacing(0);

    // Create notebook for preference categories
    let notebook = gtk4::Notebook::new();
    notebook.set_vexpand(true);
    content.append(&notebook);

    // General tab
    let (general_page, scrollback_spin, confirm_switch, copy_select_switch) =
        create_general_preferences(config);
    notebook.append_page(&general_page, Some(&Label::new(Some("General"))));

    // Appearance tab
    let (
        appearance_page,
        theme_combo,
        font_entry,
        size_spin,
        cursor_combo,
        blink_switch,
        opacity_scale,
        bold_switch,
    ) = create_appearance_preferences(config);
    notebook.append_page(&appearance_page, Some(&Label::new(Some("Appearance"))));

    // Tabs tab
    let (tabs_page, show_combo, position_combo, new_combo, close_switch) =
        create_tabs_preferences(config);
    notebook.append_page(&tabs_page, Some(&Label::new(Some("Tabs"))));

    // Shortcuts tab
    let (shortcuts_page, shortcut_entries) = create_shortcuts_preferences(config);
    notebook.append_page(&shortcuts_page, Some(&Label::new(Some("Shortcuts"))));

    // Git Sync tab
    let (
        git_sync_page,
        git_remote_entry,
        git_status_label,
        git_branch_label,
        git_last_sync_label,
        git_changes_label,
        sync_button,
    ) = create_git_sync_preferences();
    notebook.append_page(&git_sync_page, Some(&Label::new(Some("Git Sync"))));

    let on_save_callback: Rc<RefCell<Option<Box<dyn Fn(Config)>>>> =
        Rc::new(RefCell::new(Some(Box::new(on_save))));
    let base_config = Rc::new(RefCell::new(config.clone()));

    let widgets = Rc::new(PreferencesWidgets {
        scrollback_spin,
        confirm_switch,
        copy_select_switch,
        theme_combo,
        font_entry,
        size_spin,
        cursor_combo,
        blink_switch,
        opacity_scale,
        bold_switch,
        show_combo,
        position_combo,
        new_combo,
        close_switch,
        shortcut_entries,
        git_remote_entry,
        git_status_label,
        git_branch_label,
        git_last_sync_label,
        git_changes_label,
        on_save_callback: Rc::clone(&on_save_callback),
        base_config: Rc::clone(&base_config),
    });

    // Connect sync button
    let widgets_for_sync = Rc::clone(&widgets);
    sync_button.connect_clicked(move |_| {
        widgets_for_sync.sync_now();
    });

    let widgets_for_response = Rc::clone(&widgets);
    dialog.connect_response(move |dialog, response| match response {
        ResponseType::Ok | ResponseType::Apply => {
            let final_config =
                widgets_for_response.collect_config(&widgets_for_response.base_config.borrow());

            // Save config and sync if git is configured
            if let Err(e) = cterm_app::config::save_config(&final_config) {
                log::error!("Failed to save config: {}", e);
            }

            // If git sync is configured, commit and push
            if let Some(dir) = config_dir() {
                if git_sync::is_git_repo(&dir) && git_sync::get_remote_url(&dir).is_some() {
                    if let Err(e) = git_sync::commit_and_push(&dir, "Update configuration") {
                        log::error!("Failed to push config: {}", e);
                    }
                }
            }

            if let Some(ref callback) = *widgets_for_response.on_save_callback.borrow() {
                callback(final_config);
            }
            if response == ResponseType::Ok {
                dialog.close();
            }
        }
        _ => {
            dialog.close();
        }
    });

    dialog.present();
}

fn create_general_preferences(config: &Config) -> (GtkBox, SpinButton, Switch, Switch) {
    let page = GtkBox::new(Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let grid = Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(12);

    // Scrollback lines
    let scrollback_label = Label::new(Some("Scrollback lines:"));
    scrollback_label.set_halign(Align::End);
    grid.attach(&scrollback_label, 0, 0, 1, 1);

    let scrollback_spin = SpinButton::with_range(0.0, 100000.0, 1000.0);
    scrollback_spin.set_value(config.general.scrollback_lines as f64);
    grid.attach(&scrollback_spin, 1, 0, 1, 1);

    // Confirm close
    let confirm_label = Label::new(Some("Confirm close with running processes:"));
    confirm_label.set_halign(Align::End);
    grid.attach(&confirm_label, 0, 1, 1, 1);

    let confirm_switch = Switch::new();
    confirm_switch.set_active(config.general.confirm_close_with_running);
    confirm_switch.set_halign(Align::Start);
    grid.attach(&confirm_switch, 1, 1, 1, 1);

    // Copy on select
    let copy_select_label = Label::new(Some("Copy on select:"));
    copy_select_label.set_halign(Align::End);
    grid.attach(&copy_select_label, 0, 2, 1, 1);

    let copy_select_switch = Switch::new();
    copy_select_switch.set_active(config.general.copy_on_select);
    copy_select_switch.set_halign(Align::Start);
    grid.attach(&copy_select_switch, 1, 2, 1, 1);

    page.append(&grid);
    (page, scrollback_spin, confirm_switch, copy_select_switch)
}

fn create_appearance_preferences(
    config: &Config,
) -> (
    GtkBox,
    ComboBoxText,
    Entry,
    SpinButton,
    ComboBoxText,
    Switch,
    gtk4::Scale,
    Switch,
) {
    let page = GtkBox::new(Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let grid = Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(12);

    // Theme
    let theme_label = Label::new(Some("Theme:"));
    theme_label.set_halign(Align::End);
    grid.attach(&theme_label, 0, 0, 1, 1);

    let theme_combo = ComboBoxText::new();
    theme_combo.append(Some("dark"), "Default Dark");
    theme_combo.append(Some("light"), "Default Light");
    theme_combo.append(Some("tokyo_night"), "Tokyo Night");
    theme_combo.append(Some("dracula"), "Dracula");
    theme_combo.append(Some("nord"), "Nord");
    theme_combo.set_active_id(Some(&config.appearance.theme));
    grid.attach(&theme_combo, 1, 0, 1, 1);

    // Font family
    let font_label = Label::new(Some("Font:"));
    font_label.set_halign(Align::End);
    grid.attach(&font_label, 0, 1, 1, 1);

    let font_entry = Entry::new();
    font_entry.set_text(&config.appearance.font.family);
    font_entry.set_hexpand(true);
    grid.attach(&font_entry, 1, 1, 1, 1);

    // Font size
    let size_label = Label::new(Some("Font size:"));
    size_label.set_halign(Align::End);
    grid.attach(&size_label, 0, 2, 1, 1);

    let size_spin = SpinButton::with_range(6.0, 72.0, 1.0);
    size_spin.set_value(config.appearance.font.size);
    grid.attach(&size_spin, 1, 2, 1, 1);

    // Cursor style
    let cursor_label = Label::new(Some("Cursor style:"));
    cursor_label.set_halign(Align::End);
    grid.attach(&cursor_label, 0, 3, 1, 1);

    let cursor_combo = ComboBoxText::new();
    cursor_combo.append(Some("block"), "Block");
    cursor_combo.append(Some("underline"), "Underline");
    cursor_combo.append(Some("bar"), "Bar");
    let cursor_id = match config.appearance.cursor_style {
        CursorStyleConfig::Block => "block",
        CursorStyleConfig::Underline => "underline",
        CursorStyleConfig::Bar => "bar",
    };
    cursor_combo.set_active_id(Some(cursor_id));
    grid.attach(&cursor_combo, 1, 3, 1, 1);

    // Cursor blink
    let blink_label = Label::new(Some("Cursor blink:"));
    blink_label.set_halign(Align::End);
    grid.attach(&blink_label, 0, 4, 1, 1);

    let blink_switch = Switch::new();
    blink_switch.set_active(config.appearance.cursor_blink);
    blink_switch.set_halign(Align::Start);
    grid.attach(&blink_switch, 1, 4, 1, 1);

    // Opacity
    let opacity_label = Label::new(Some("Opacity:"));
    opacity_label.set_halign(Align::End);
    grid.attach(&opacity_label, 0, 5, 1, 1);

    let opacity_scale = gtk4::Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 0.1);
    opacity_scale.set_value(config.appearance.opacity);
    opacity_scale.set_hexpand(true);
    grid.attach(&opacity_scale, 1, 5, 1, 1);

    // Bold is bright
    let bold_label = Label::new(Some("Bold text uses bright colors:"));
    bold_label.set_halign(Align::End);
    grid.attach(&bold_label, 0, 6, 1, 1);

    let bold_switch = Switch::new();
    bold_switch.set_active(config.appearance.bold_is_bright);
    bold_switch.set_halign(Align::Start);
    grid.attach(&bold_switch, 1, 6, 1, 1);

    page.append(&grid);
    (
        page,
        theme_combo,
        font_entry,
        size_spin,
        cursor_combo,
        blink_switch,
        opacity_scale,
        bold_switch,
    )
}

fn create_tabs_preferences(
    config: &Config,
) -> (GtkBox, ComboBoxText, ComboBoxText, ComboBoxText, Switch) {
    let page = GtkBox::new(Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let grid = Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(12);

    // Show tab bar
    let show_label = Label::new(Some("Show tab bar:"));
    show_label.set_halign(Align::End);
    grid.attach(&show_label, 0, 0, 1, 1);

    let show_combo = ComboBoxText::new();
    show_combo.append(Some("always"), "Always");
    show_combo.append(Some("multiple"), "When multiple tabs");
    show_combo.append(Some("never"), "Never");
    let show_id = match config.tabs.show_tab_bar {
        TabBarVisibility::Always => "always",
        TabBarVisibility::Multiple => "multiple",
        TabBarVisibility::Never => "never",
    };
    show_combo.set_active_id(Some(show_id));
    grid.attach(&show_combo, 1, 0, 1, 1);

    // Tab bar position
    let position_label = Label::new(Some("Tab bar position:"));
    position_label.set_halign(Align::End);
    grid.attach(&position_label, 0, 1, 1, 1);

    let position_combo = ComboBoxText::new();
    position_combo.append(Some("top"), "Top");
    position_combo.append(Some("bottom"), "Bottom");
    let position_id = match config.tabs.tab_bar_position {
        TabBarPosition::Top => "top",
        TabBarPosition::Bottom => "bottom",
    };
    position_combo.set_active_id(Some(position_id));
    grid.attach(&position_combo, 1, 1, 1, 1);

    // New tab position
    let new_label = Label::new(Some("New tab position:"));
    new_label.set_halign(Align::End);
    grid.attach(&new_label, 0, 2, 1, 1);

    let new_combo = ComboBoxText::new();
    new_combo.append(Some("end"), "At end");
    new_combo.append(Some("after_current"), "After current");
    let new_id = match config.tabs.new_tab_position {
        NewTabPosition::End => "end",
        NewTabPosition::AfterCurrent => "after_current",
    };
    new_combo.set_active_id(Some(new_id));
    grid.attach(&new_combo, 1, 2, 1, 1);

    // Show close button
    let close_label = Label::new(Some("Show close button:"));
    close_label.set_halign(Align::End);
    grid.attach(&close_label, 0, 3, 1, 1);

    let close_switch = Switch::new();
    close_switch.set_active(config.tabs.show_close_button);
    close_switch.set_halign(Align::Start);
    grid.attach(&close_switch, 1, 3, 1, 1);

    page.append(&grid);
    (page, show_combo, position_combo, new_combo, close_switch)
}

fn create_shortcuts_preferences(config: &Config) -> (GtkBox, Vec<(String, Entry)>) {
    let page = GtkBox::new(Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let label = Label::new(Some("Keyboard Shortcuts"));
    label.set_halign(Align::Start);
    label.add_css_class("heading");
    page.append(&label);

    let scroll = ScrolledWindow::new();
    scroll.set_vexpand(true);

    let grid = Grid::new();
    grid.set_row_spacing(4);
    grid.set_column_spacing(12);

    let shortcuts = [
        ("new_tab", "New Tab", &config.shortcuts.new_tab),
        ("close_tab", "Close Tab", &config.shortcuts.close_tab),
        ("next_tab", "Next Tab", &config.shortcuts.next_tab),
        ("prev_tab", "Previous Tab", &config.shortcuts.prev_tab),
        ("new_window", "New Window", &config.shortcuts.new_window),
        (
            "close_window",
            "Close Window",
            &config.shortcuts.close_window,
        ),
        ("copy", "Copy", &config.shortcuts.copy),
        ("paste", "Paste", &config.shortcuts.paste),
        ("select_all", "Select All", &config.shortcuts.select_all),
        ("zoom_in", "Zoom In", &config.shortcuts.zoom_in),
        ("zoom_out", "Zoom Out", &config.shortcuts.zoom_out),
        ("zoom_reset", "Zoom Reset", &config.shortcuts.zoom_reset),
        ("find", "Find", &config.shortcuts.find),
        ("reset", "Reset", &config.shortcuts.reset),
    ];

    let mut entries = Vec::new();

    for (i, (key, name, shortcut)) in shortcuts.iter().enumerate() {
        let name_label = Label::new(Some(*name));
        name_label.set_halign(Align::End);
        grid.attach(&name_label, 0, i as i32, 1, 1);

        let shortcut_entry = Entry::new();
        shortcut_entry.set_text(shortcut);
        shortcut_entry.set_hexpand(true);
        grid.attach(&shortcut_entry, 1, i as i32, 1, 1);

        entries.push((key.to_string(), shortcut_entry));
    }

    scroll.set_child(Some(&grid));
    page.append(&scroll);
    (page, entries)
}

fn create_git_sync_preferences() -> (GtkBox, Entry, Label, Label, Label, Label, Button) {
    let page = GtkBox::new(Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    // Get sync status
    let status = config_dir()
        .map(|dir| git_sync::get_sync_status(&dir))
        .unwrap_or_default();

    // Remote Repository section
    let remote_header = Label::new(Some("Remote Repository"));
    remote_header.set_halign(Align::Start);
    remote_header.add_css_class("heading");
    page.append(&remote_header);

    let remote_grid = Grid::new();
    remote_grid.set_row_spacing(8);
    remote_grid.set_column_spacing(12);

    // Git remote URL
    let git_label = Label::new(Some("Git Remote URL:"));
    git_label.set_halign(Align::End);
    remote_grid.attach(&git_label, 0, 0, 1, 1);

    let git_entry = Entry::new();
    git_entry.set_placeholder_text(Some("https://github.com/user/cterm-config.git"));
    git_entry.set_hexpand(true);
    if let Some(url) = &status.remote_url {
        git_entry.set_text(url);
    }
    remote_grid.attach(&git_entry, 1, 0, 1, 1);

    page.append(&remote_grid);

    // Status section
    let status_header = Label::new(Some("Sync Status"));
    status_header.set_halign(Align::Start);
    status_header.set_margin_top(12);
    status_header.add_css_class("heading");
    page.append(&status_header);

    let status_grid = Grid::new();
    status_grid.set_row_spacing(8);
    status_grid.set_column_spacing(12);

    // Status
    let status_label_title = Label::new(Some("Status:"));
    status_label_title.set_halign(Align::End);
    status_grid.attach(&status_label_title, 0, 0, 1, 1);

    let status_text = if !status.is_repo {
        "Not initialized"
    } else if status.remote_url.is_none() {
        "No remote configured"
    } else {
        "Configured"
    };
    let git_status_label = Label::new(Some(status_text));
    git_status_label.set_halign(Align::Start);
    status_grid.attach(&git_status_label, 1, 0, 1, 1);

    // Branch
    let branch_label_title = Label::new(Some("Branch:"));
    branch_label_title.set_halign(Align::End);
    status_grid.attach(&branch_label_title, 0, 1, 1, 1);

    let branch_text = status.branch.clone().unwrap_or_else(|| "-".to_string());
    let git_branch_label = Label::new(Some(&branch_text));
    git_branch_label.set_halign(Align::Start);
    status_grid.attach(&git_branch_label, 1, 1, 1, 1);

    // Last sync
    let last_sync_label_title = Label::new(Some("Last sync:"));
    last_sync_label_title.set_halign(Align::End);
    status_grid.attach(&last_sync_label_title, 0, 2, 1, 1);

    let last_sync_text = if let Some(ts) = status.last_commit_time {
        format_timestamp(ts)
    } else {
        "-".to_string()
    };
    let git_last_sync_label = Label::new(Some(&last_sync_text));
    git_last_sync_label.set_halign(Align::Start);
    status_grid.attach(&git_last_sync_label, 1, 2, 1, 1);

    // Changes
    let changes_label_title = Label::new(Some("Changes:"));
    changes_label_title.set_halign(Align::End);
    status_grid.attach(&changes_label_title, 0, 3, 1, 1);

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
    let git_changes_label = Label::new(Some(changes_text));
    git_changes_label.set_halign(Align::Start);
    status_grid.attach(&git_changes_label, 1, 3, 1, 1);

    page.append(&status_grid);

    // Sync Now button
    let button_box = GtkBox::new(Orientation::Horizontal, 12);
    button_box.set_margin_top(12);

    let sync_button = Button::with_label("Sync Now");
    button_box.append(&sync_button);

    page.append(&button_box);

    (
        page,
        git_entry,
        git_status_label,
        git_branch_label,
        git_last_sync_label,
        git_changes_label,
        sync_button,
    )
}
