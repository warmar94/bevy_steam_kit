//! Pure helpers: connect strings.

/// `"<prefix> <lobby>"` - the connect string used for rich presence and invites.
pub fn connect_string(prefix: &str, lobby: u64) -> String {
    format!("{prefix} {lobby}")
}

/// Find `<prefix> <lobby>` (whitespace separated) anywhere in `text` - a rich-presence connect
/// string, a full command line, or process arguments joined with spaces - and return the lobby
/// id. `<prefix>=<lobby>` is accepted too. `None` when missing, malformed, or `0`.
///
/// Only the FIRST `<prefix>` token counts: when its value is malformed the result is `None`, even
/// if a valid `<prefix> <lobby>` follows it in `text`.
pub fn parse_connect_lobby(text: &str, prefix: &str) -> Option<u64> {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return None;
    }
    let mut tokens = text.split_whitespace();
    while let Some(tok) = tokens.next() {
        let tok = tok.trim_matches('"');
        let value = if tok == prefix {
            tokens.next()?
        } else if let Some(rest) = tok.strip_prefix(prefix).and_then(|r| r.strip_prefix('=')) {
            rest
        } else {
            continue;
        };
        return value.trim_matches('"').parse::<u64>().ok().filter(|&id| id != 0);
    }
    None
}
