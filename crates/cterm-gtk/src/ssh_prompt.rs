//! Native GTK dialogs for interactive SSH prompts (host key, password,
//! passphrase) — used both by the in-process tunnel
//! (`DaemonConnection::connect_ssh_with_prompts`) and by daemon-side SSH tabs
//! (which deliver prompts over gRPC as `SessionPromptEvent`).

use std::sync::Arc;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    ButtonsType, DialogFlags, MessageDialog, MessageType, PasswordEntry, ResponseType, Window,
};

use cterm_proto::proto::{PromptKind, SessionPromptEvent};

/// Interactive prompt callbacks backed by native GTK dialogs, for the in-process
/// SSH tunnel. The callbacks run on an off-main connection thread; each marshals
/// its dialog onto the GTK main loop and blocks for the answer.
pub fn interactive_prompts() -> cterm_core::SshPrompts {
    cterm_core::SshPrompts {
        host_key: Some(Arc::new(|req: cterm_core::HostKeyRequest| {
            let kind = if req.changed {
                PromptKind::HostkeyChanged
            } else {
                PromptKind::HostkeyUnknown
            };
            host_key_dialog(kind, &req.host, req.port, &req.key_type, &req.fingerprint)
        })),
        password: Some(Arc::new(|text: &str| {
            secret_dialog("Password required", text)
        })),
        passphrase: Some(Arc::new(|text: &str| {
            secret_dialog("Key passphrase required", text)
        })),
    }
}

/// Show the appropriate dialog for a daemon-delivered prompt and return
/// `(accept, secret)`.
pub fn show_ssh_prompt(prompt: &SessionPromptEvent) -> (bool, Option<String>) {
    match PromptKind::try_from(prompt.kind).unwrap_or(PromptKind::Unspecified) {
        PromptKind::HostkeyUnknown | PromptKind::HostkeyChanged => {
            let kind = PromptKind::try_from(prompt.kind).unwrap_or(PromptKind::HostkeyUnknown);
            let accept = host_key_dialog(
                kind,
                &prompt.host,
                prompt.port as u16,
                &prompt.key_type,
                &prompt.fingerprint,
            );
            (accept, None)
        }
        PromptKind::Password => (false, secret_dialog("Password required", &prompt.text)),
        PromptKind::Passphrase => (
            false,
            secret_dialog("Key passphrase required", &prompt.text),
        ),
        PromptKind::Unspecified => (false, None),
    }
}

fn host_key_dialog(
    kind: PromptKind,
    host: &str,
    port: u16,
    key_type: &str,
    fingerprint: &str,
) -> bool {
    let changed = kind == PromptKind::HostkeyChanged;
    let warning = if changed {
        "WARNING: the host key for this server has CHANGED since you last \
         connected. This could indicate a man-in-the-middle attack.\n\n"
    } else {
        "The authenticity of this host can't be established.\n\n"
    };
    let body = format!(
        "{warning}Host: {host}:{port}\nKey type: {key_type}\nFingerprint: {fingerprint}\n\n\
         Do you want to continue connecting and trust this key?"
    );

    run_on_main_blocking(move |tx| {
        let dialog = MessageDialog::new(
            None::<&Window>,
            DialogFlags::MODAL,
            if changed {
                MessageType::Error
            } else {
                MessageType::Warning
            },
            ButtonsType::None,
            &body,
        );
        dialog.add_button("Cancel", ResponseType::Cancel);
        dialog.add_button("Trust & Connect", ResponseType::Accept);
        dialog.connect_response(move |d, resp| {
            let _ = tx.send(resp == ResponseType::Accept);
            d.close();
        });
        dialog.present();
    })
    .unwrap_or(false)
}

fn secret_dialog(title: &str, text: &str) -> Option<String> {
    let prompt_text = if text.is_empty() {
        title.to_string()
    } else {
        text.to_string()
    };

    run_on_main_blocking(move |tx| {
        let dialog = MessageDialog::new(
            None::<&Window>,
            DialogFlags::MODAL,
            MessageType::Question,
            ButtonsType::OkCancel,
            &prompt_text,
        );
        let entry = PasswordEntry::builder()
            .show_peek_icon(true)
            .activates_default(true)
            .build();
        dialog.content_area().append(&entry);
        dialog.set_default_response(ResponseType::Ok);
        dialog.connect_response(move |d, resp| {
            let secret = if resp == ResponseType::Ok {
                Some(entry.text().to_string())
            } else {
                None
            };
            let _ = tx.send(secret);
            d.close();
        });
        dialog.present();
    })
    .flatten()
}

/// Marshal `f` onto the GTK main loop and block (off-main) for its result.
///
/// `f` is given a sender it must fulfil exactly once (typically from the
/// dialog's response handler).
fn run_on_main_blocking<R, F>(f: F) -> Option<R>
where
    R: Send + 'static,
    F: FnOnce(std::sync::mpsc::Sender<R>) + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    glib::idle_add_once(move || f(tx));
    rx.recv().ok()
}
