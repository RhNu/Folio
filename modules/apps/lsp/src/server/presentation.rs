//! Hover rendering and URI actions at the protocol boundary.
use super::*;
use crate::protocol::{path_to_uri, percent_decode, range_json};
use folio_hir::Symbol;
use folio_project_model::{DependencyKind, SourceId};
use folio_source::SourceSpan;

/// Escape user-authored prose before placing it inside trusted navigation Markdown.
pub(super) fn prose(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\\' | '\u{60}' | '*' | '_' | '[' | ']' | '(' | ')' | '#' | '!' | '|' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

pub(super) fn encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

pub(super) fn command_link(title: &str, command: &str, arguments: Value) -> String {
    format!(
        "[{}](command:{command}?{})",
        prose(title),
        encode(&arguments.to_string())
    )
}

/// A variable-length fence keeps literal defaults inside the highlighted code.
fn code(value: &str) -> String {
    let value = readable_declaration(value);
    let longest = value.split(|ch| ch != '~').map(str::len).max().unwrap_or(0);
    let fence = "~".repeat(longest.max(2) + 1);
    format!("{fence}papyrus\n{value}\n{fence}")
}

/// Break long parameter lists at CST-equivalent token boundaries, never inside literals.
fn readable_declaration(value: &str) -> String {
    let mut lines = Vec::new();
    for line in value.lines() {
        if line.chars().count() <= 88 {
            lines.push(line.to_owned());
            continue;
        }
        let tokens = folio_papyrus::lex(line);
        let mut depth = 0usize;
        let mut open = None;
        let mut close = None;
        let mut commas = Vec::new();
        for token in tokens {
            match token.kind {
                folio_papyrus::SyntaxKind::LParen => {
                    if depth == 0 {
                        open = Some(token.range.end);
                    }
                    depth += 1;
                }
                folio_papyrus::SyntaxKind::RParen => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        close = Some(token.range.start);
                        break;
                    }
                }
                folio_papyrus::SyntaxKind::Comma if depth == 1 => commas.push(token.range.start),
                _ => {}
            }
        }
        let (Some(open), Some(close)) = (open, close) else {
            lines.push(line.to_owned());
            continue;
        };
        if commas.is_empty() {
            lines.push(line.to_owned());
            continue;
        }
        let mut result = format!("{} \\\n", &line[..open]);
        let mut start = open;
        for end in commas {
            result.push_str(&format!("    {}, \\\n", line[start..end].trim()));
            start = end + 1;
        }
        result.push_str(&format!(
            "    {} \\\n{}",
            line[start..close].trim(),
            &line[close..]
        ));
        lines.push(result);
    }
    lines.join("\n")
}

pub(super) fn hover_text(
    item: &folio_ide::Hover,
    origin: Option<&str>,
    settings: &settings::EditorSettings,
    markdown: bool,
    links: &[String],
) -> String {
    let mut context = item
        .owner_script
        .clone()
        .unwrap_or_else(|| "Papyrus".into());
    if let Some(Symbol::StateMember { state, .. }) = &item.symbol {
        context.push_str(" · ");
        context.push_str(state);
    }
    if settings.details
        && let Some(origin) = origin
    {
        context.push_str(" · ");
        context.push_str(origin);
    }
    let mut header = Vec::new();
    if settings.details {
        header.push(if markdown { prose(&context) } else { context });
    }
    header.push(if markdown {
        code(&item.declaration)
    } else {
        item.declaration.clone()
    });
    // VS Code's rule has a negative bottom margin; keep it away from the code block.
    let mut sections = vec![header.join("\n\n")];
    if settings.documentation
        && let Some(documentation) = &item.documentation
    {
        sections.push(if markdown {
            documentation
                .lines()
                .map(prose)
                .collect::<Vec<_>>()
                .join("  \n")
        } else {
            documentation.clone()
        });
    }
    if settings.details && !item.details.is_empty() {
        sections.push(
            item.details
                .iter()
                .map(|detail| {
                    if markdown {
                        prose(detail)
                    } else {
                        detail.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
        );
    }
    if settings.details && markdown && !links.is_empty() {
        sections.push(links.join(" · "));
    }
    sections.join(if markdown { "\n\n---\n\n" } else { "\n\n" })
}

pub(super) fn source_location(
    view: &ProjectAnalysisView,
    span: SourceSpan,
    encoding: PositionEncoding,
) -> Option<Value> {
    let source = view.sources.get(&span.file)?;
    let text = view.analysis.text(span.file)?;
    Some(
        json!({"uri":path_to_uri(&source.canonical_path),"range":range_json(folio_ide::range(text, span.range, encoding)?)}),
    )
}

pub(super) fn virtual_uri(owner: &str) -> String {
    format!("folio-declaration:/{}.psc", encode(owner))
}

/// Virtual names are resolved against the selected API, never as filesystem paths.
pub(super) fn virtual_owner(uri: &str) -> Option<String> {
    let encoded = uri
        .strip_prefix("folio-declaration:/")?
        .strip_suffix(".psc")?;
    let owner = percent_decode(encoded)?;
    (!owner.is_empty() && !owner.contains(['/', '\\', '?', '#'])).then_some(owner)
}

/// Retrieve only the resolver's verified PSC snapshot for the selected provider.
pub(super) fn external_source(
    metadata: &Metadata,
    loaded: &LoadedProject,
    owner: &str,
    overlays: &BTreeMap<PathBuf, Arc<str>>,
) -> Option<(PathBuf, String)> {
    let selected = &metadata
        .scripts
        .iter()
        .find(|item| item.script.eq_ignore_ascii_case(owner))?
        .selected;
    let dependency = loaded.dependencies.iter().find(|dependency| {
        dependency.source_id == selected.package.source && dependency.kind == DependencyKind::Psc
    })?;
    let path = dependency
        .canonical_path
        .join(selected.declaration.as_ref()?.source_path.as_ref()?);
    if let Some(text) = overlays.get(&path) {
        return Some((path, text.to_string()));
    }
    let bytes = loaded
        .input_snapshots
        .iter()
        .find_map(|snapshot| match snapshot {
            folio_project_resolve::io::InputSnapshot::File {
                path: candidate,
                bytes,
            } if candidate == &path => Some(bytes),
            _ => None,
        })?;
    let SourceId::Dependency { index, .. } = dependency.source_id else {
        return None;
    };
    let encoding = loaded.root.manifest.dependencies.get(index)?.encoding;
    let text = folio_project_resolve::io::decode_source(bytes, encoding).ok()?;
    Some((path, text))
}

#[cfg(test)]
mod tests;
