//! Manage Remotes dialog for GTK4
//!
//! Simple dialog for adding/removing remote hosts that templates can target.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, ComboBoxText, Dialog, Entry, Grid, Label, Orientation,
    ResponseType, Window,
};

use cterm_app::config::{load_config, save_config, Config, ConnectionMethod, RemoteConfig};

/// Show the Manage Remotes dialog
pub fn show_remotes_dialog<F>(parent: &impl IsA<Window>, on_save: F)
where
    F: Fn() + 'static,
{
    let config = load_config().unwrap_or_default();
    let config = Rc::new(RefCell::new(config));

    let dialog = Dialog::builder()
        .title("Manage Remotes")
        .transient_for(parent)
        .modal(true)
        .default_width(400)
        .default_height(220)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("Save", ResponseType::Ok);

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Top row: popup + add/remove buttons
    let top_row = GtkBox::new(Orientation::Horizontal, 8);
    let remote_combo = ComboBoxText::new();
    remote_combo.set_hexpand(true);
    top_row.append(&remote_combo);

    let add_btn = Button::with_label("+");
    let remove_btn = Button::with_label("\u{2212}"); // minus sign
    top_row.append(&add_btn);
    top_row.append(&remove_btn);
    content.append(&top_row);

    // Name / Host fields
    let grid = Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(12);

    let name_label = Label::new(Some("Name:"));
    name_label.set_halign(Align::End);
    grid.attach(&name_label, 0, 0, 1, 1);
    let name_entry = Entry::new();
    name_entry.set_placeholder_text(Some("my-server"));
    name_entry.set_hexpand(true);
    grid.attach(&name_entry, 1, 0, 1, 1);

    let host_label = Label::new(Some("Host:"));
    host_label.set_halign(Align::End);
    grid.attach(&host_label, 0, 1, 1, 1);
    let host_entry = Entry::new();
    host_entry.set_placeholder_text(Some("user@hostname"));
    host_entry.set_hexpand(true);
    grid.attach(&host_entry, 1, 1, 1, 1);

    let method_label = Label::new(Some("Method:"));
    method_label.set_halign(Align::End);
    grid.attach(&method_label, 0, 2, 1, 1);
    let method_combo = ComboBoxText::new();
    method_combo.append_text("ctermd");
    method_combo.append_text("Mosh");
    method_combo.set_active(Some(0));
    grid.attach(&method_combo, 1, 2, 1, 1);

    let proxy_label = Label::new(Some("Proxy Jump:"));
    proxy_label.set_halign(Align::End);
    grid.attach(&proxy_label, 0, 3, 1, 1);
    let proxy_entry = Entry::new();
    proxy_entry.set_placeholder_text(Some("relay.example.com"));
    proxy_entry.set_hexpand(true);
    grid.attach(&proxy_entry, 1, 3, 1, 1);

    content.append(&grid);

    // Wrap entries for sharing across closures
    let name_entry = Rc::new(name_entry);
    let host_entry = Rc::new(host_entry);
    let method_combo = Rc::new(method_combo);
    let proxy_entry = Rc::new(proxy_entry);
    let remote_combo = Rc::new(remote_combo);

    // Populate combo
    {
        let cfg = config.borrow();
        populate_combo(&remote_combo, &cfg.remotes);
        if !cfg.remotes.is_empty() {
            remote_combo.set_active(Some(0));
        }
    }

    // Load selected remote into fields
    fn load_selected(
        combo: &ComboBoxText,
        config: &Rc<RefCell<Config>>,
        name_entry: &Entry,
        host_entry: &Entry,
        method_combo: &ComboBoxText,
        proxy_entry: &Entry,
    ) {
        let idx = combo.active().map(|i| i as usize);
        let cfg = config.borrow();
        if let Some(idx) = idx {
            if let Some(remote) = cfg.remotes.get(idx) {
                name_entry.set_text(&remote.name);
                host_entry.set_text(&remote.host);
                method_combo.set_active(Some(match remote.method {
                    ConnectionMethod::Daemon => 0,
                    ConnectionMethod::Mosh => 1,
                }));
                proxy_entry.set_text(remote.proxy_jump.as_deref().unwrap_or(""));
                return;
            }
        }
        name_entry.set_text("");
        host_entry.set_text("");
        method_combo.set_active(Some(0));
        proxy_entry.set_text("");
    }

    fn save_current(
        combo: &ComboBoxText,
        config: &Rc<RefCell<Config>>,
        name_entry: &Entry,
        host_entry: &Entry,
        method_combo: &ComboBoxText,
        proxy_entry: &Entry,
    ) {
        let idx = combo.active().map(|i| i as usize);
        if let Some(idx) = idx {
            let mut cfg = config.borrow_mut();
            if let Some(remote) = cfg.remotes.get_mut(idx) {
                remote.name = name_entry.text().to_string();
                remote.host = host_entry.text().to_string();
                remote.method = match method_combo.active() {
                    Some(1) => ConnectionMethod::Mosh,
                    _ => ConnectionMethod::Daemon,
                };
                let proxy = proxy_entry.text().to_string();
                remote.proxy_jump = if proxy.is_empty() { None } else { Some(proxy) };
            }
        }
    }

    fn populate_combo(combo: &ComboBoxText, remotes: &[RemoteConfig]) {
        combo.remove_all();
        for remote in remotes {
            combo.append_text(&format!("{} ({})", remote.name, remote.host));
        }
        if remotes.is_empty() {
            combo.append_text("(no remotes)");
        }
    }

    // Load initial selection
    load_selected(
        &remote_combo,
        &config,
        &name_entry,
        &host_entry,
        &method_combo,
        &proxy_entry,
    );

    // Combo changed → load selected
    {
        let config = Rc::clone(&config);
        let name_entry = Rc::clone(&name_entry);
        let host_entry = Rc::clone(&host_entry);
        let method_combo = Rc::clone(&method_combo);
        let proxy_entry = Rc::clone(&proxy_entry);
        let combo = Rc::clone(&remote_combo);
        remote_combo.connect_changed(move |_| {
            load_selected(
                &combo,
                &config,
                &name_entry,
                &host_entry,
                &method_combo,
                &proxy_entry,
            );
        });
    }

    // Name/host/method/proxy changed → save current to config and refresh combo
    {
        let config = Rc::clone(&config);
        let name_entry_c = Rc::clone(&name_entry);
        let host_entry_c = Rc::clone(&host_entry);
        let method_combo_c = Rc::clone(&method_combo);
        let proxy_entry_c = Rc::clone(&proxy_entry);
        let combo = Rc::clone(&remote_combo);
        let update = move || {
            save_current(
                &combo,
                &config,
                &name_entry_c,
                &host_entry_c,
                &method_combo_c,
                &proxy_entry_c,
            );
            let idx = combo.active();
            let cfg = config.borrow();
            populate_combo(&combo, &cfg.remotes);
            if let Some(idx) = idx {
                combo.set_active(Some(idx));
            }
        };
        let update = Rc::new(update);

        let u = Rc::clone(&update);
        name_entry.connect_changed(move |_| u());
        let u = Rc::clone(&update);
        host_entry.connect_changed(move |_| u());
        let u = Rc::clone(&update);
        method_combo.connect_changed(move |_| u());
        let u = Rc::clone(&update);
        proxy_entry.connect_changed(move |_| u());
    }

    // Add button
    {
        let config = Rc::clone(&config);
        let combo = Rc::clone(&remote_combo);
        let name_entry = Rc::clone(&name_entry);
        let host_entry = Rc::clone(&host_entry);
        let method_combo = Rc::clone(&method_combo);
        let proxy_entry = Rc::clone(&proxy_entry);
        add_btn.connect_clicked(move |_| {
            let mut cfg = config.borrow_mut();
            let name = format!("remote-{}", cfg.remotes.len() + 1);
            cfg.remotes.push(RemoteConfig {
                name,
                host: String::new(),
                method: Default::default(),
                proxy_jump: None,
            });
            let new_idx = cfg.remotes.len() - 1;
            populate_combo(&combo, &cfg.remotes);
            combo.set_active(Some(new_idx as u32));
            drop(cfg);
            load_selected(
                &combo,
                &config,
                &name_entry,
                &host_entry,
                &method_combo,
                &proxy_entry,
            );
        });
    }

    // Remove button
    {
        let config = Rc::clone(&config);
        let combo = Rc::clone(&remote_combo);
        let name_entry = Rc::clone(&name_entry);
        let host_entry = Rc::clone(&host_entry);
        let method_combo = Rc::clone(&method_combo);
        let proxy_entry = Rc::clone(&proxy_entry);
        remove_btn.connect_clicked(move |_| {
            if let Some(idx) = combo.active() {
                let mut cfg = config.borrow_mut();
                let idx = idx as usize;
                if idx < cfg.remotes.len() {
                    cfg.remotes.remove(idx);
                }
                populate_combo(&combo, &cfg.remotes);
                if !cfg.remotes.is_empty() {
                    combo.set_active(Some(0));
                }
                drop(cfg);
                load_selected(
                    &combo,
                    &config,
                    &name_entry,
                    &host_entry,
                    &method_combo,
                    &proxy_entry,
                );
            }
        });
    }

    // Handle response
    let config_for_save = Rc::clone(&config);
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Ok {
            let cfg = config_for_save.borrow();
            if let Err(e) = save_config(&cfg) {
                log::error!("Failed to save config: {}", e);
            }
            on_save();
        }
        dialog.close();
    });

    dialog.present();
}
