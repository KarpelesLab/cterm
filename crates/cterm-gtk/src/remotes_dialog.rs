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

use cterm_app::config::{
    load_config, save_config, Config, ConnectionMethod, ConnectionType, RemoteConfig,
};

/// All form fields wrapped for sharing across closures.
struct Fields {
    name: Rc<Entry>,
    host: Rc<Entry>,
    method: Rc<ComboBoxText>,
    conn_type: Rc<ComboBoxText>,
    proxy: Rc<Entry>,
    relay_user: Rc<Entry>,
    relay_device: Rc<Entry>,
    session_name: Rc<Entry>,
    remote_combo: Rc<ComboBoxText>,
}

impl Fields {
    fn load(&self, config: &Rc<RefCell<Config>>) {
        let idx = self.remote_combo.active().map(|i| i as usize);
        let cfg = config.borrow();
        if let Some(idx) = idx {
            if let Some(remote) = cfg.remotes.get(idx) {
                self.name.set_text(&remote.name);
                self.host.set_text(&remote.host);
                self.method.set_active(Some(match remote.method {
                    ConnectionMethod::Daemon => 0,
                    ConnectionMethod::Mosh => 1,
                }));
                self.conn_type
                    .set_active(Some(match remote.connection_type {
                        ConnectionType::Direct => 0,
                        ConnectionType::Relay => 1,
                    }));
                self.proxy
                    .set_text(remote.proxy_jump.as_deref().unwrap_or(""));
                self.relay_user
                    .set_text(remote.relay_username.as_deref().unwrap_or(""));
                self.relay_device
                    .set_text(remote.relay_device.as_deref().unwrap_or(""));
                self.session_name
                    .set_text(remote.session_name.as_deref().unwrap_or(""));
                return;
            }
        }
        self.name.set_text("");
        self.host.set_text("");
        self.method.set_active(Some(0));
        self.conn_type.set_active(Some(0));
        self.proxy.set_text("");
        self.relay_user.set_text("");
        self.relay_device.set_text("");
        self.session_name.set_text("");
    }

    fn save(&self, config: &Rc<RefCell<Config>>) {
        let idx = self.remote_combo.active().map(|i| i as usize);
        if let Some(idx) = idx {
            let mut cfg = config.borrow_mut();
            if let Some(remote) = cfg.remotes.get_mut(idx) {
                remote.name = self.name.text().to_string();
                remote.host = self.host.text().to_string();
                remote.method = match self.method.active() {
                    Some(1) => ConnectionMethod::Mosh,
                    _ => ConnectionMethod::Daemon,
                };
                remote.connection_type = match self.conn_type.active() {
                    Some(1) => ConnectionType::Relay,
                    _ => ConnectionType::Direct,
                };
                set_opt(&self.proxy, &mut remote.proxy_jump);
                set_opt(&self.relay_user, &mut remote.relay_username);
                set_opt(&self.relay_device, &mut remote.relay_device);
                set_opt(&self.session_name, &mut remote.session_name);
            }
        }
    }
}

fn set_opt(entry: &Entry, target: &mut Option<String>) {
    let val = entry.text().to_string();
    *target = if val.is_empty() { None } else { Some(val) };
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
        .default_height(380)
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

    // Fields grid
    let grid = Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(12);

    let mut row = 0;

    let name_entry = Entry::new();
    name_entry.set_placeholder_text(Some("my-server"));
    name_entry.set_hexpand(true);
    attach_label(&grid, "Name:", row);
    grid.attach(&name_entry, 1, row, 1, 1);
    row += 1;

    let host_entry = Entry::new();
    host_entry.set_placeholder_text(Some("user@hostname"));
    host_entry.set_hexpand(true);
    attach_label(&grid, "Host:", row);
    grid.attach(&host_entry, 1, row, 1, 1);
    row += 1;

    let method_combo = ComboBoxText::new();
    method_combo.append_text("ctermd");
    method_combo.append_text("Mosh");
    method_combo.set_active(Some(0));
    attach_label(&grid, "Method:", row);
    grid.attach(&method_combo, 1, row, 1, 1);
    row += 1;

    let conn_type_combo = ComboBoxText::new();
    conn_type_combo.append_text("Direct");
    conn_type_combo.append_text("Relay");
    conn_type_combo.set_active(Some(0));
    attach_label(&grid, "Type:", row);
    grid.attach(&conn_type_combo, 1, row, 1, 1);
    row += 1;

    let proxy_entry = Entry::new();
    proxy_entry.set_placeholder_text(Some("unixshells.com"));
    proxy_entry.set_hexpand(true);
    attach_label(&grid, "Proxy/Relay:", row);
    grid.attach(&proxy_entry, 1, row, 1, 1);
    row += 1;

    let relay_user_entry = Entry::new();
    relay_user_entry.set_placeholder_text(Some("username"));
    relay_user_entry.set_hexpand(true);
    attach_label(&grid, "Relay User:", row);
    grid.attach(&relay_user_entry, 1, row, 1, 1);
    row += 1;

    let relay_device_entry = Entry::new();
    relay_device_entry.set_placeholder_text(Some("device-name"));
    relay_device_entry.set_hexpand(true);
    attach_label(&grid, "Device:", row);
    grid.attach(&relay_device_entry, 1, row, 1, 1);
    row += 1;

    let session_name_entry = Entry::new();
    session_name_entry.set_placeholder_text(Some("default"));
    session_name_entry.set_hexpand(true);
    attach_label(&grid, "Session:", row);
    grid.attach(&session_name_entry, 1, row, 1, 1);

    content.append(&grid);

    let fields = Rc::new(Fields {
        name: Rc::new(name_entry),
        host: Rc::new(host_entry),
        method: Rc::new(method_combo),
        conn_type: Rc::new(conn_type_combo),
        proxy: Rc::new(proxy_entry),
        relay_user: Rc::new(relay_user_entry),
        relay_device: Rc::new(relay_device_entry),
        session_name: Rc::new(session_name_entry),
        remote_combo: Rc::new(remote_combo),
    });

    // Populate combo
    {
        let cfg = config.borrow();
        populate_combo(&fields.remote_combo, &cfg.remotes);
        if !cfg.remotes.is_empty() {
            fields.remote_combo.set_active(Some(0));
        }
    }

    // Load initial selection
    fields.load(&config);

    // Combo changed → load selected
    {
        let config = Rc::clone(&config);
        let fields_cb = Rc::clone(&fields);
        fields
            .remote_combo
            .connect_changed(move |_| fields_cb.load(&config));
    }

    // Field changes → save and refresh combo
    {
        let config = Rc::clone(&config);
        let fields_up = Rc::clone(&fields);
        let update = move || {
            fields_up.save(&config);
            let idx = fields_up.remote_combo.active();
            let cfg = config.borrow();
            populate_combo(&fields_up.remote_combo, &cfg.remotes);
            if let Some(idx) = idx {
                fields_up.remote_combo.set_active(Some(idx));
            }
        };
        let update = Rc::new(update);

        let u = Rc::clone(&update);
        fields.name.connect_changed(move |_| u());
        let u = Rc::clone(&update);
        fields.host.connect_changed(move |_| u());
        let u = Rc::clone(&update);
        fields.method.connect_changed(move |_| u());
        let u = Rc::clone(&update);
        fields.conn_type.connect_changed(move |_| u());
        let u = Rc::clone(&update);
        fields.proxy.connect_changed(move |_| u());
        let u = Rc::clone(&update);
        fields.relay_user.connect_changed(move |_| u());
        let u = Rc::clone(&update);
        fields.relay_device.connect_changed(move |_| u());
        let u = Rc::clone(&update);
        fields.session_name.connect_changed(move |_| u());
    }

    // Add button
    {
        let config = Rc::clone(&config);
        let fields = Rc::clone(&fields);
        add_btn.connect_clicked(move |_| {
            let mut cfg = config.borrow_mut();
            let name = format!("remote-{}", cfg.remotes.len() + 1);
            cfg.remotes.push(RemoteConfig {
                name,
                host: String::new(),
                method: Default::default(),
                connection_type: Default::default(),
                proxy_jump: None,
                relay_username: None,
                relay_device: None,
                session_name: None,
            });
            let new_idx = cfg.remotes.len() - 1;
            populate_combo(&fields.remote_combo, &cfg.remotes);
            fields.remote_combo.set_active(Some(new_idx as u32));
            drop(cfg);
            fields.load(&config);
        });
    }

    // Remove button
    {
        let config = Rc::clone(&config);
        let fields = Rc::clone(&fields);
        remove_btn.connect_clicked(move |_| {
            if let Some(idx) = fields.remote_combo.active() {
                let mut cfg = config.borrow_mut();
                let idx = idx as usize;
                if idx < cfg.remotes.len() {
                    cfg.remotes.remove(idx);
                }
                populate_combo(&fields.remote_combo, &cfg.remotes);
                if !cfg.remotes.is_empty() {
                    fields.remote_combo.set_active(Some(0));
                }
                drop(cfg);
                fields.load(&config);
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

fn attach_label(grid: &Grid, text: &str, row: i32) {
    let label = Label::new(Some(text));
    label.set_halign(Align::End);
    grid.attach(&label, 0, row, 1, 1);
}
