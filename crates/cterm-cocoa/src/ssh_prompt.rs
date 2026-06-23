//! Native macOS dialogs for interactive SSH prompts (host key, password,
//! passphrase) raised by the daemon while establishing a native SSH session.

use std::sync::Arc;

use objc2::MainThreadOnly;
use objc2_app_kit::{NSAlert, NSAlertStyle, NSSecureTextField};
use objc2_foundation::{MainThreadMarker, NSRect, NSSize, NSString};

use cterm_proto::proto::{PromptKind, SessionPromptEvent};

/// Build interactive prompt callbacks backed by native dialogs, for the
/// in-process SSH tunnel (`DaemonConnection::connect_ssh_with_prompts`).
///
/// The callbacks run on the connection's (off-main) blocking thread; each one
/// dispatches its dialog to the main thread and blocks for the user's answer.
pub fn interactive_prompts() -> cterm_core::SshPrompts {
    cterm_core::SshPrompts {
        host_key: Some(Arc::new(|req: cterm_core::HostKeyRequest| {
            let kind = if req.changed {
                PromptKind::HostkeyChanged
            } else {
                PromptKind::HostkeyUnknown
            };
            let event = SessionPromptEvent {
                prompt_id: String::new(),
                kind: kind as i32,
                host: req.host,
                port: req.port as u32,
                key_type: req.key_type,
                fingerprint: req.fingerprint,
                text: String::new(),
            };
            run_on_main_blocking(move |mtm| show_ssh_prompt(mtm, &event))
                .map(|(accept, _)| accept)
                .unwrap_or(false)
        })),
        password: Some(Arc::new(|text: &str| {
            secret_via_dialog(PromptKind::Password, text)
        })),
        passphrase: Some(Arc::new(|text: &str| {
            secret_via_dialog(PromptKind::Passphrase, text)
        })),
    }
}

fn secret_via_dialog(kind: PromptKind, text: &str) -> Option<String> {
    let event = SessionPromptEvent {
        prompt_id: String::new(),
        kind: kind as i32,
        host: String::new(),
        port: 0,
        key_type: String::new(),
        fingerprint: String::new(),
        text: text.to_string(),
    };
    run_on_main_blocking(move |mtm| show_ssh_prompt(mtm, &event)).and_then(|(_, secret)| secret)
}

/// Run `f` on the main thread and block (off-main) until it returns.
fn run_on_main_blocking<F, R>(f: F) -> Option<R>
where
    F: FnOnce(MainThreadMarker) -> R + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    dispatch2::Queue::main().exec_async(move || {
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let _ = tx.send(f(mtm));
    });
    rx.recv().ok()
}

/// Show the appropriate dialog for `prompt` and return `(accept, secret)`.
///
/// For host-key prompts, `accept` reflects the user's choice and `secret` is
/// `None`. For password/passphrase prompts, `secret` is the entered text when
/// the user confirms, or `None` if they cancel (`accept` is then `false`).
///
/// Must be called on the main thread.
pub fn show_ssh_prompt(
    mtm: MainThreadMarker,
    prompt: &SessionPromptEvent,
) -> (bool, Option<String>) {
    match PromptKind::try_from(prompt.kind).unwrap_or(PromptKind::Unspecified) {
        PromptKind::HostkeyUnknown | PromptKind::HostkeyChanged => host_key_dialog(mtm, prompt),
        PromptKind::Password | PromptKind::Passphrase => secret_dialog(mtm, prompt),
        PromptKind::Unspecified => (false, None),
    }
}

fn host_key_dialog(mtm: MainThreadMarker, prompt: &SessionPromptEvent) -> (bool, Option<String>) {
    let changed = PromptKind::try_from(prompt.kind) == Ok(PromptKind::HostkeyChanged);

    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(if changed {
        NSAlertStyle::Critical
    } else {
        NSAlertStyle::Warning
    });

    let title = if changed {
        "Host key has CHANGED"
    } else {
        "Unknown host key"
    };
    alert.setMessageText(&NSString::from_str(title));

    let warning = if changed {
        "WARNING: the host key for this server has changed since you last \
         connected. This could indicate a man-in-the-middle attack.\n\n"
    } else {
        "The authenticity of this host can't be established.\n\n"
    };
    let body = format!(
        "{warning}Host: {}:{}\nKey type: {}\nFingerprint: {}\n\n\
         Do you want to continue connecting and trust this key?",
        prompt.host, prompt.port, prompt.key_type, prompt.fingerprint,
    );
    alert.setInformativeText(&NSString::from_str(&body));

    alert.addButtonWithTitle(&NSString::from_str("Trust & Connect"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    let response = alert.runModal();
    let accept = response == objc2_app_kit::NSAlertFirstButtonReturn;
    (accept, None)
}

fn secret_dialog(mtm: MainThreadMarker, prompt: &SessionPromptEvent) -> (bool, Option<String>) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);

    let title = if PromptKind::try_from(prompt.kind) == Ok(PromptKind::Passphrase) {
        "Key passphrase required"
    } else {
        "Password required"
    };
    alert.setMessageText(&NSString::from_str(title));
    let text = if prompt.text.is_empty() {
        "Enter your password:".to_string()
    } else {
        prompt.text.clone()
    };
    alert.setInformativeText(&NSString::from_str(&text));

    // Secure text field accessory for entry.
    let field = unsafe {
        let frame = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            NSSize::new(260.0, 24.0),
        );
        let f = NSSecureTextField::initWithFrame(NSSecureTextField::alloc(mtm), frame);
        f
    };
    alert.setAccessoryView(Some(&field));

    alert.addButtonWithTitle(&NSString::from_str("OK"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    let response = alert.runModal();
    if response == objc2_app_kit::NSAlertFirstButtonReturn {
        let value = unsafe { field.stringValue() };
        (true, Some(value.to_string()))
    } else {
        (false, None)
    }
}
