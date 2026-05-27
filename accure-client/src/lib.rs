//! Shared library for `accure-client` and `accure-send`.
//!
//! Exports the command parser so both binaries can use it without
//! duplicating code.

use accure_core::messages::ClientCommand;
use accure_core::op::Right;

/// Parse a single user command line. Returns `Ok(cmd)` if recognized.
pub fn parse_command(line: &str) -> Result<ClientCommand, ParseError> {
    let mut parts = line.split_whitespace();
    let head = parts.next().ok_or(ParseError::Empty)?.to_ascii_lowercase();
    match head.as_str() {
        "insert" | "i" => {
            let pos = parts
                .next()
                .ok_or(ParseError::Usage("insert <pos> <char>"))?
                .parse::<usize>()
                .map_err(|_| ParseError::Usage("insert <pos> <char>"))?;
            let ch_str = parts.next().ok_or(ParseError::Usage("insert <pos> <char>"))?;
            let ch = ch_str
                .chars()
                .next()
                .ok_or(ParseError::Usage("insert <pos> <char>"))?;
            Ok(ClientCommand::Insert { pos, ch })
        }
        "delete" | "del" | "d" | "x" => {
            let pos = parts
                .next()
                .ok_or(ParseError::Usage("delete <pos>"))?
                .parse::<usize>()
                .map_err(|_| ParseError::Usage("delete <pos>"))?;
            Ok(ClientCommand::Delete { pos })
        }
        "allow" => {
            let target = parts
                .next()
                .ok_or(ParseError::Usage("allow <site> <a|r|w>"))?
                .to_string();
            let right = Right::parse(
                parts
                    .next()
                    .ok_or(ParseError::Usage("allow <site> <a|r|w>"))?,
            )
            .ok_or(ParseError::Usage("right must be a|r|w"))?;
            Ok(ClientCommand::Allow { target, right })
        }
        "deny" => {
            let target = parts
                .next()
                .ok_or(ParseError::Usage("deny <site> <a|r|w>"))?
                .to_string();
            let right = Right::parse(
                parts
                    .next()
                    .ok_or(ParseError::Usage("deny <site> <a|r|w>"))?,
            )
            .ok_or(ParseError::Usage("right must be a|r|w"))?;
            Ok(ClientCommand::Deny { target, right })
        }
        "snapshot" | "s" | "refresh" => Ok(ClientCommand::Snapshot),
        _ => Err(ParseError::Unknown(head)),
    }
}

#[derive(Debug)]
pub enum ParseError {
    Empty,
    Unknown(String),
    Usage(&'static str),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Empty => write!(f, "empty command"),
            ParseError::Unknown(s) => write!(f, "unknown command: {s}"),
            ParseError::Usage(u) => write!(f, "usage: {u}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_insert_short_and_long() {
        match parse_command("insert 3 x").unwrap() {
            ClientCommand::Insert { pos, ch } => assert!(pos == 3 && ch == 'x'),
            _ => panic!(),
        }
        match parse_command("i 0 a").unwrap() {
            ClientCommand::Insert { pos, ch } => assert!(pos == 0 && ch == 'a'),
            _ => panic!(),
        }
    }

    #[test]
    fn parses_policy_commands() {
        match parse_command("allow S2 w").unwrap() {
            ClientCommand::Allow { target, right } => {
                assert_eq!(target, "S2");
                assert_eq!(right, Right::Write);
            }
            _ => panic!(),
        }
        match parse_command("deny B admin").unwrap() {
            ClientCommand::Deny { target, right } => {
                assert_eq!(target, "B");
                assert_eq!(right, Right::Admin);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn rejects_unknown_and_partial() {
        assert!(matches!(parse_command(""), Err(ParseError::Empty)));
        assert!(matches!(parse_command("foo"), Err(ParseError::Unknown(_))));
        assert!(matches!(parse_command("insert"), Err(ParseError::Usage(_))));
        assert!(matches!(parse_command("insert x x"), Err(ParseError::Usage(_))));
    }
}
