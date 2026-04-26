//! Session picker dialog for attaching to daemon sessions

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Dialog, Label, ListBox, ListBoxRow, Orientation, ResponseType,
    ScrolledWindow, Window,
};

/// Information about a daemon session for display
pub struct SessionEntry {
    pub session_id: String,
    pub title: String,
    pub cols: u32,
    pub rows: u32,
    pub running: bool,
}

/// Show the session picker dialog
///
/// Connects to the local daemon, lists available sessions, and lets the user
/// pick one to attach to. The callback receives the session ID.
pub fn show_session_picker<F>(parent: &impl IsA<Window>, callback: F)
where
    F: Fn(String) + 'static,
{
    let dialog = Dialog::builder()
        .title("Attach to Session")
        .transient_for(parent)
        .modal(true)
        .default_width(500)
        .default_height(350)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("Attach", ResponseType::Ok);

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let info_label = Label::new(Some("Select a daemon session to attach to:"));
    info_label.set_halign(Align::Start);
    content.append(&info_label);

    // Session list
    let scrolled = ScrolledWindow::builder()
        .vexpand(true)
        .min_content_height(200)
        .build();

    let list_box = ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    scrolled.set_child(Some(&list_box));
    content.append(&scrolled);

    // Status label for loading/errors
    let status_label = Label::new(Some("Connecting to daemon..."));
    status_label.set_halign(Align::Start);
    content.append(&status_label);

    // Refresh button
    let refresh_btn = Button::with_label("Refresh");
    refresh_btn.set_halign(Align::End);
    content.append(&refresh_btn);

    // Store session data
    let sessions_data: Rc<RefCell<Vec<SessionEntry>>> = Rc::new(RefCell::new(Vec::new()));

    // Load sessions initially
    {
        let list_box = list_box.clone();
        let sessions_data = Rc::clone(&sessions_data);
        let status_label = status_label.clone();
        load_sessions(&list_box, &sessions_data, &status_label);
    }

    // Refresh button handler
    {
        let list_box = list_box.clone();
        let sessions_data = Rc::clone(&sessions_data);
        let status_label = status_label.clone();
        refresh_btn.connect_clicked(move |_| {
            load_sessions(&list_box, &sessions_data, &status_label);
        });
    }

    // Handle response
    let sessions_data_resp = Rc::clone(&sessions_data);
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Ok {
            if let Some(row) = list_box.selected_row() {
                let idx = row.index() as usize;
                let sessions = sessions_data_resp.borrow();
                if let Some(session) = sessions.get(idx) {
                    callback(session.session_id.clone());
                }
            }
        }
        dialog.close();
    });

    dialog.present();
}

/// Load sessions from the daemon in a background thread
fn load_sessions(
    list_box: &ListBox,
    sessions_data: &Rc<RefCell<Vec<SessionEntry>>>,
    status_label: &Label,
) {
    // Clear existing entries
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }
    sessions_data.borrow_mut().clear();
    status_label.set_text("Connecting to daemon...");

    let list_box = list_box.clone();
    let sessions_data = Rc::clone(sessions_data);
    let status_label = status_label.clone();

    // Spawn background thread to query daemon
    let (tx, rx) = std::sync::mpsc::channel::<SessionResult>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();

        let result = match rt {
            Ok(rt) => rt.block_on(async {
                let conn = cterm_client::DaemonConnection::connect_local().await?;
                let sessions = conn.list_sessions().await?;
                let entries: Vec<SessionEntry> = sessions
                    .into_iter()
                    .map(|s| SessionEntry {
                        session_id: s.session_id,
                        title: s.title,
                        cols: s.cols,
                        rows: s.rows,
                        running: s.running,
                    })
                    .collect();
                Ok(entries)
            }),
            Err(e) => Err(cterm_client::ClientError::Connection(e.to_string())),
        };

        let _ = tx.send(result);
    });

    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(entries) => {
                        if entries.is_empty() {
                            status_label.set_text(
                                "No sessions available. The daemon is running but has no active sessions.",
                            );
                        } else {
                            status_label.set_text(&format!("{} session(s) found", entries.len()));
                            for entry in &entries {
                                let row = create_session_row(entry);
                                list_box.append(&row);
                            }
                            // Select first row
                            if let Some(first_row) = list_box.row_at_index(0) {
                                list_box.select_row(Some(&first_row));
                            }
                        }
                        *sessions_data.borrow_mut() = entries;
                    }
                    Err(e) => {
                        status_label.set_text(&format!("Failed to connect: {}", e));
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

type SessionResult = std::result::Result<Vec<SessionEntry>, cterm_client::ClientError>;

/// Create a list row for a session entry
fn create_session_row(session: &SessionEntry) -> ListBoxRow {
    let row = ListBoxRow::new();

    let hbox = GtkBox::new(Orientation::Horizontal, 12);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);

    // Status indicator
    let status = if session.running { "Running" } else { "Exited" };
    let status_label = Label::new(Some(status));
    if session.running {
        status_label.add_css_class("success");
    } else {
        status_label.add_css_class("dim-label");
    }
    hbox.append(&status_label);

    // Title and details
    let details_box = GtkBox::new(Orientation::Vertical, 2);
    details_box.set_hexpand(true);

    let title = if session.title.is_empty() {
        "Untitled"
    } else {
        &session.title
    };
    let title_label = Label::new(Some(title));
    title_label.set_halign(Align::Start);
    title_label.add_css_class("heading");
    details_box.append(&title_label);

    let info = format!(
        "{}x{} - {}",
        session.cols,
        session.rows,
        &session.session_id[..8.min(session.session_id.len())]
    );
    let info_label = Label::new(Some(&info));
    info_label.set_halign(Align::Start);
    info_label.add_css_class("dim-label");
    details_box.append(&info_label);

    hbox.append(&details_box);
    row.set_child(Some(&hbox));
    row
}

/// Show the SSH connection dialog.
///
/// Prompts the user for a hostname (user@host), connects via SSH, attaches to
/// all existing sessions on the remote daemon, and creates one new session.
/// The callback receives a list of reconnected sessions (existing + new).
pub fn show_ssh_dialog<F>(parent: &impl IsA<Window>, callback: F)
where
    F: Fn(Vec<cterm_app::daemon_reconnect::ReconnectedSession>) + 'static,
{
    let callback = Rc::new(callback);
    let dialog = Dialog::builder()
        .title("SSH Remote Terminal")
        .transient_for(parent)
        .modal(true)
        .default_width(400)
        .default_height(150)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("Connect", ResponseType::Ok);

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let label = Label::new(Some("Host (e.g. user@hostname or user@hostname:port):"));
    label.set_halign(Align::Start);
    content.append(&label);

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("user@hostname:port"));
    entry.set_activates_default(true);
    content.append(&entry);

    let status_label = Label::new(None);
    status_label.set_halign(Align::Start);
    content.append(&status_label);

    dialog.set_default_response(ResponseType::Ok);

    dialog.connect_response(move |dialog, response| {
        if response != ResponseType::Ok {
            dialog.close();
            return;
        }

        let host = entry.text().to_string();
        if host.is_empty() {
            status_label.set_text("Please enter a hostname.");
            return;
        }

        status_label.set_text("Connecting...");

        let (tx, rx) = std::sync::mpsc::channel::<SshConnectResult>();
        let host_bg = host.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();

            let result = match rt {
                Ok(rt) => rt.block_on(async {
                    let (conn, _tunnel) =
                        cterm_client::DaemonConnection::connect_ssh(&host_bg, true).await?;

                    // Attach to all existing running sessions
                    let mut sessions =
                        cterm_app::daemon_reconnect::reconnect_all_sessions_on(&conn).await?;

                    // If no sessions exist, create one
                    if sessions.is_empty() {
                        let handle = conn
                            .create_session(cterm_client::CreateSessionOpts {
                                cols: 80,
                                rows: 24,
                                ..Default::default()
                            })
                            .await?;
                        sessions.push(cterm_app::daemon_reconnect::ReconnectedSession {
                            handle,
                            title: String::new(),
                            custom_title: String::new(),
                            tab_color: String::new(),
                            template_name: String::new(),
                            screen: None,
                        });
                    }

                    Ok(sessions)
                }),
                Err(e) => Err(cterm_client::ClientError::Connection(e.to_string())),
            };

            let _ = tx.send(result);
        });

        let dialog_weak = dialog.downgrade();
        let callback = Rc::clone(&callback);
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(sessions) => {
                            callback(sessions);
                            if let Some(dialog) = dialog_weak.upgrade() {
                                dialog.close();
                            }
                        }
                        Err(e) => {
                            if let Some(dialog) = dialog_weak.upgrade() {
                                let content = dialog.content_area();
                                let mut child = content.first_child();
                                let mut last_label = None;
                                while let Some(widget) = child {
                                    if widget.downcast_ref::<Label>().is_some() {
                                        last_label = Some(widget.clone());
                                    }
                                    child = widget.next_sibling();
                                }
                                if let Some(label) = last_label {
                                    if let Some(label) = label.downcast_ref::<Label>() {
                                        label.set_text(&format!("Connection failed: {}", e));
                                    }
                                }
                            }
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    });

    dialog.present();
}

type SshConnectResult = std::result::Result<
    Vec<cterm_app::daemon_reconnect::ReconnectedSession>,
    cterm_client::ClientError,
>;
