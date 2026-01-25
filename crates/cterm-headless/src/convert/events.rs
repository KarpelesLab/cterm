//! Terminal event conversion between cterm-core and proto

use crate::proto;
use cterm_core::screen::{
    ClipboardOperation as CoreClipboardOp, ClipboardSelection as CoreClipboardSel,
};
use cterm_core::term::TerminalEvent as CoreEvent;

/// Convert cterm_core TerminalEvent to proto TerminalEvent
pub fn event_to_proto(event: &CoreEvent) -> proto::TerminalEvent {
    use proto::terminal_event::Event;

    let event = match event {
        CoreEvent::TitleChanged(title) => Event::TitleChanged(proto::TitleChangedEvent {
            title: title.clone(),
        }),
        CoreEvent::Bell => Event::Bell(proto::BellEvent {}),
        CoreEvent::ProcessExited(code) => {
            Event::ProcessExited(proto::ProcessExitedEvent { exit_code: *code })
        }
        CoreEvent::ContentChanged => Event::ContentChanged(proto::ContentChangedEvent {}),
        CoreEvent::ClipboardRequest(op) => {
            let (operation, selection, data) = match op {
                CoreClipboardOp::Query { selection } => (
                    proto::ClipboardOperation::Read,
                    selection_to_proto(selection),
                    None,
                ),
                CoreClipboardOp::Set { selection, data } => (
                    proto::ClipboardOperation::Write,
                    selection_to_proto(selection),
                    Some(data.clone()),
                ),
            };
            Event::ClipboardRequest(proto::ClipboardRequestEvent {
                operation: operation as i32,
                selection: selection as i32,
                data,
            })
        }
    };

    proto::TerminalEvent { event: Some(event) }
}

/// Convert clipboard selection to proto
fn selection_to_proto(sel: &CoreClipboardSel) -> proto::ClipboardSelection {
    match sel {
        CoreClipboardSel::Clipboard => proto::ClipboardSelection::Clipboard,
        CoreClipboardSel::Primary => proto::ClipboardSelection::Primary,
        CoreClipboardSel::Select => proto::ClipboardSelection::Select,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_changed_event() {
        let event = CoreEvent::TitleChanged("test".to_string());
        let proto = event_to_proto(&event);
        match proto.event {
            Some(proto::terminal_event::Event::TitleChanged(e)) => {
                assert_eq!(e.title, "test");
            }
            _ => panic!("Expected TitleChanged event"),
        }
    }

    #[test]
    fn test_bell_event() {
        let event = CoreEvent::Bell;
        let proto = event_to_proto(&event);
        assert!(matches!(
            proto.event,
            Some(proto::terminal_event::Event::Bell(_))
        ));
    }

    #[test]
    fn test_process_exited_event() {
        let event = CoreEvent::ProcessExited(42);
        let proto = event_to_proto(&event);
        match proto.event {
            Some(proto::terminal_event::Event::ProcessExited(e)) => {
                assert_eq!(e.exit_code, 42);
            }
            _ => panic!("Expected ProcessExited event"),
        }
    }
}
