use super::models::{ApprovalCommand, PermissionBehavior};

pub fn parse_approval_command(content: &str) -> Option<ApprovalCommand> {
    let mut parts = content.split_whitespace();
    let command = parts.next()?.to_ascii_lowercase();
    let approval_id = parts.next()?.to_string();
    if parts.next().is_some() {
        return None;
    }
    match command.as_str() {
        "/allow" => Some(ApprovalCommand {
            decision: PermissionBehavior::Allow,
            approval_id,
        }),
        "/deny" => Some(ApprovalCommand {
            decision: PermissionBehavior::Deny,
            approval_id,
        }),
        _ => None,
    }
}

pub fn new_approval_id() -> String {
    use rand::RngCore;
    let mut bytes = [0_u8; 4];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!(
        "apv_{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3]
    )
}
