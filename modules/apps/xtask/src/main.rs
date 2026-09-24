//! Workspace maintenance commands for Folio contributors.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use tracing::{debug, info};

const WARN_LINES: usize = 650;
const ERROR_LINES: usize = 1200;

#[derive(Parser)]
#[command(about = "Folio workspace maintenance tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check non-blank, non-comment Rust source lines in modules.
    Lines,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Ok,
    Warning,
    Error,
}

impl Level {
    fn for_lines(lines: usize) -> Self {
        if lines > ERROR_LINES {
            Self::Error
        } else if lines > WARN_LINES {
            Self::Warning
        } else {
            Self::Ok
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum LexState {
    Code,
    BlockComment(usize),
    String,
    Char,
    RawString(usize),
}

/// Count lines with Rust content, preserving comment markers inside literals.
fn code_lines(source: &str) -> usize {
    let bytes = source.as_bytes();
    let mut state = LexState::Code;
    let mut index = 0;
    let mut lines = 0;
    let mut has_code = false;

    while index < bytes.len() {
        if bytes[index] == b'\n' {
            lines += usize::from(has_code);
            has_code = matches!(
                state,
                LexState::String | LexState::Char | LexState::RawString(_)
            );
            index += 1;
            continue;
        }

        match state {
            LexState::Code => {
                if bytes[index..].starts_with(b"//") {
                    index = bytes[index..]
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .map_or(bytes.len(), |offset| index + offset);
                } else if bytes[index..].starts_with(b"/*") {
                    state = LexState::BlockComment(1);
                    index += 2;
                } else if let Some((prefix_len, hashes)) = raw_string_start(bytes, index) {
                    has_code = true;
                    state = LexState::RawString(hashes);
                    index += prefix_len;
                } else if bytes[index] == b'"' {
                    has_code = true;
                    state = LexState::String;
                    index += 1;
                } else if bytes[index] == b'\'' && char_literal_end(bytes, index).is_some() {
                    has_code = true;
                    state = LexState::Char;
                    index += 1;
                } else {
                    has_code |= !bytes[index].is_ascii_whitespace();
                    index += 1;
                }
            }
            LexState::BlockComment(depth) => {
                if bytes[index..].starts_with(b"/*") {
                    state = LexState::BlockComment(depth + 1);
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    state = if depth == 1 {
                        LexState::Code
                    } else {
                        LexState::BlockComment(depth - 1)
                    };
                    index += 2;
                } else {
                    index += 1;
                }
            }
            LexState::String | LexState::Char => {
                has_code = true;
                if bytes[index] == b'\\' && bytes.get(index + 1).is_some_and(|next| *next != b'\n')
                {
                    index += 2;
                } else {
                    let closing = if matches!(state, LexState::String) {
                        b'"'
                    } else {
                        b'\''
                    };
                    if bytes[index] == closing {
                        state = LexState::Code;
                    }
                    index += 1;
                }
            }
            LexState::RawString(hashes) => {
                has_code = true;
                if bytes[index] == b'"'
                    && bytes[index + 1..]
                        .get(..hashes)
                        .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
                {
                    state = LexState::Code;
                    index += hashes + 1;
                } else {
                    index += 1;
                }
            }
        }
    }

    lines + usize::from(has_code)
}

/// Recognize ordinary and byte raw string prefixes before interpreting comments.
fn raw_string_start(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
    let prefix = match bytes.get(index..index + 2) {
        Some(b"br" | b"cr") => 2,
        _ if bytes[index] == b'r' => 1,
        _ => return None,
    };
    let mut cursor = index + prefix;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    (bytes.get(cursor) == Some(&b'"')).then_some((cursor - index + 1, cursor - index - prefix))
}

/// Lifetimes have no closing quote; only treat a quote as a character literal when it closes here.
fn char_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start + 1;
    while let Some(&byte) = bytes.get(cursor) {
        if byte == b'\n' {
            return None;
        }
        if byte == b'\\' {
            cursor += 2;
        } else if byte == b'\'' {
            let body = std::str::from_utf8(&bytes[start + 1..cursor]).ok()?;
            return (body.starts_with('\\') || body.chars().count() == 1).then_some(cursor);
        } else {
            cursor += 1;
        }
    }
    None
}

/// Visit only real directories so linked trees cannot escape or duplicate the module scan.
fn rust_sources(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", directory.display()))
        })?;
        for entry in entries {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "rs") {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Report every threshold breach and fail when any source file requires splitting.
fn check_lines() -> io::Result<ExitCode> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("xtask lives in modules/apps/xtask");
    let files = rust_sources(&workspace.join("modules"))?;
    let mut warnings = 0;
    let mut errors = 0;
    for file in &files {
        let source = fs::read_to_string(file).map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", file.display()))
        })?;
        let count = code_lines(&source);
        let relative = file.strip_prefix(workspace).unwrap_or(file);
        debug!(path = %relative.display(), lines = count, "counted source lines");
        match Level::for_lines(count) {
            Level::Ok => {}
            Level::Warning => {
                warnings += 1;
                println!(
                    "warning: {}: {count} lines (limit {WARN_LINES})",
                    relative.display()
                );
            }
            Level::Error => {
                errors += 1;
                println!(
                    "error: {}: {count} lines (limit {ERROR_LINES}; split this file)",
                    relative.display()
                );
            }
        }
    }
    println!(
        "Checked {} Rust files: {warnings} warnings, {errors} errors",
        files.len()
    );
    info!(
        files = files.len(),
        warnings, errors, "source line check complete"
    );
    Ok(if errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    match Cli::parse().command {
        Command::Lines => match check_lines() {
            Ok(code) => code,
            Err(error) => {
                eprintln!("xtask lines: {error}");
                ExitCode::FAILURE
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Level, code_lines};

    #[test]
    fn counts_code_around_nested_comments_and_literals() {
        let source = "// comment\n/* outer\n /* inner */\n still comment */ let x = 1; // tail\nlet url = \"// literal\";\nlet raw = r#\"/* literal\nstill literal\"#;\n";
        assert_eq!(code_lines(source), 4);
    }

    #[test]
    fn recognizes_char_literals_and_lifetimes() {
        assert_eq!(
            code_lines("let slash = '/'; // tail\nlet x: &'a str = \"x\";\n"),
            2
        );
        assert_eq!(code_lines("let a: &'a str = y; // tail\n"), 1);
    }

    #[test]
    fn counts_empty_lines_inside_multiline_literals() {
        assert_eq!(code_lines("let text = r#\"first\n\nlast\"#;\n"), 3);
        assert_eq!(code_lines("let text = \"first\\\nlast\";\n"), 2);
    }

    #[test]
    fn thresholds_are_strict() {
        assert_eq!(Level::for_lines(650), Level::Ok);
        assert_eq!(Level::for_lines(651), Level::Warning);
        assert_eq!(Level::for_lines(1200), Level::Warning);
        assert_eq!(Level::for_lines(1201), Level::Error);
    }
}
