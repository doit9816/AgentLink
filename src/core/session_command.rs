#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCommand {
    New { label: Option<String> },
    List,
    Switch { slot_id: String },
    Current,
    Dir(DirCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirCommand {
    Show,
    Reset,
    Back,
    Set(String),
}

pub fn parse_session_command(content: &str) -> Option<SessionCommand> {
    let trimmed = content.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let mut parts = trimmed.split_whitespace();
    let head = parts.next()?.to_ascii_lowercase();
    match head.as_str() {
        "/new" => {
            let label = parts.collect::<Vec<_>>().join(" ");
            let label = label.trim();
            Some(SessionCommand::New {
                label: if label.is_empty() {
                    None
                } else {
                    Some(label.to_string())
                },
            })
        }
        "/list" | "/ls" => Some(SessionCommand::List),
        "/switch" => {
            let slot_id = parts.next()?.to_string();
            Some(SessionCommand::Switch { slot_id })
        }
        "/current" => Some(SessionCommand::Current),
        "/dir" | "/cd" => {
            let arg = parts.next();
            Some(SessionCommand::Dir(match arg {
                None => DirCommand::Show,
                Some("reset") => DirCommand::Reset,
                Some("-") => DirCommand::Back,
                Some(value) => DirCommand::Set(value.to_string()),
            }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_commands() {
        assert_eq!(
            parse_session_command("/new my task"),
            Some(SessionCommand::New {
                label: Some("my task".to_string())
            })
        );
        assert_eq!(parse_session_command("/list"), Some(SessionCommand::List));
        assert_eq!(
            parse_session_command("/switch 2"),
            Some(SessionCommand::Switch {
                slot_id: "2".to_string()
            })
        );
        assert_eq!(
            parse_session_command("/cd /tmp"),
            Some(SessionCommand::Dir(DirCommand::Set("/tmp".to_string())))
        );
        assert!(parse_session_command("hello").is_none());
    }
}
