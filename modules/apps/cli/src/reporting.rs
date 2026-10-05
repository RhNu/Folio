//! Text and JSON reports for command results and diagnostics.
use std::io::Write;

use folio_build::{ProjectAnalysisView, execute, output};
use folio_diagnostics::{Diagnostic, Severity};
use folio_project_model::{
    DependencyEdge, ExternalRequirement, Metadata, PackageId, ScriptSelection,
};
use folio_source::SourceSpan;
use serde::Serialize;

use super::CliError;

pub(super) fn print_build(
    output: &mut impl Write,
    outcome: &execute::BuildOutcome,
) -> Result<(), CliError> {
    print_success(output, &outcome.success)?;
    if outcome.unchanged {
        writeln!(output, "  inputs and published output unchanged").map_err(CliError::Output)?;
    }
    for (unit, decision) in &outcome.cache_decisions {
        writeln!(output, "  cache {unit}: {decision:?}").map_err(CliError::Output)?;
    }
    Ok(())
}

pub(super) fn write_build_json(
    output: &mut impl Write,
    outcome: &execute::BuildOutcome,
    diagnostics: &CheckReport,
) -> Result<(), CliError> {
    let cache = outcome
        .cache_decisions
        .iter()
        .map(|(unit, decision)| {
            let (status, reason) = match decision {
                output::CacheDecision::Hit => ("hit", None),
                output::CacheDecision::Missing => ("miss", Some("not present")),
                output::CacheDecision::Invalid(reason) => ("miss", Some(*reason)),
            };
            serde_json::json!({"unit_id": unit, "status": status, "reason": reason})
        })
        .collect::<Vec<_>>();
    let report = serde_json::json!({"schema": 4, "success": outcome.success, "cache": cache, "unchanged": outcome.unchanged, "diagnostics": diagnostics.diagnostics});
    write_json(output, &report)
}

pub(super) fn print_success(
    output: &mut impl Write,
    success: &output::SuccessRecord,
) -> Result<(), CliError> {
    writeln!(output, "{} build unit(s) succeeded", success.units.len())
        .map_err(CliError::Output)?;
    for unit in &success.units {
        writeln!(
            output,
            "  {} {} {} -> {}",
            unit.package, unit.package_version, unit.target, success.output
        )
        .map_err(CliError::Output)?;
        for artifact in &unit.artifacts {
            writeln!(
                output,
                "    {} <- {}:{}",
                artifact.path, artifact.source_package, artifact.source_path
            )
            .map_err(CliError::Output)?;
        }
    }
    for requirement in &success.external_requirements {
        writeln!(
            output,
            "  external {} {}: {}",
            requirement.package,
            requirement.package_version.as_deref().unwrap_or(""),
            requirement.reason
        )
        .map_err(CliError::Output)?;
    }
    Ok(())
}

#[derive(Serialize)]
pub(super) struct CheckReport {
    schema: u32,
    target: String,
    diagnostics: Vec<CheckDiagnostic>,
}

#[derive(Serialize)]
struct CheckDiagnostic {
    code: String,
    severity: &'static str,
    message: String,
    primary: Option<CheckLocation>,
    related: Vec<CheckRelatedLocation>,
}

#[derive(Serialize)]
struct CheckLocation {
    path: String,
    start_byte: usize,
    end_byte: usize,
    line: usize,
    byte_column: usize,
    column: usize,
}

#[derive(Serialize)]
struct CheckRelatedLocation {
    message: String,
    location: Option<CheckLocation>,
}

impl CheckReport {
    pub(super) fn new(
        target: &str,
        view: &ProjectAnalysisView,
        diagnostics: &[Diagnostic],
    ) -> Self {
        let diagnostics = diagnostics
            .iter()
            .map(|diagnostic| CheckDiagnostic {
                code: diagnostic.code.clone(),
                severity: match diagnostic.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                    Severity::Info => "info",
                },
                message: diagnostic.message.clone(),
                primary: diagnostic
                    .primary
                    .and_then(|span| check_location(view, span)),
                related: diagnostic
                    .related
                    .iter()
                    .map(|related| CheckRelatedLocation {
                        message: related.message.clone(),
                        location: check_location(view, related.span),
                    })
                    .collect(),
            })
            .collect();
        Self {
            schema: 1,
            target: target.to_owned(),
            diagnostics,
        }
    }
}

fn check_location(view: &ProjectAnalysisView, span: SourceSpan) -> Option<CheckLocation> {
    let source = view.sources.get(&span.file)?;
    let text = view.analysis.text(span.file)?;
    let (line, byte_column) = view
        .analysis
        .line_index(span.file)?
        .line_col(span.range.start)?;
    let column = text
        .get(span.range.start - byte_column..span.range.start)?
        .chars()
        .count();
    Some(CheckLocation {
        path: source.display_path.clone(),
        start_byte: span.range.start,
        end_byte: span.range.end,
        line,
        byte_column,
        column,
    })
}

pub(super) fn print_check(
    output: &mut impl Write,
    report: &CheckReport,
    command: &str,
) -> Result<(), CliError> {
    for diagnostic in &report.diagnostics {
        if let Some(location) = &diagnostic.primary {
            writeln!(
                output,
                "{}:{}:{}: {}[{}]: {}",
                location.path,
                location.line + 1,
                location.column + 1,
                diagnostic.severity,
                diagnostic.code,
                diagnostic.message
            )
            .map_err(CliError::Output)?;
        } else {
            writeln!(
                output,
                "{}[{}]: {}",
                diagnostic.severity, diagnostic.code, diagnostic.message
            )
            .map_err(CliError::Output)?;
        }
        for related in &diagnostic.related {
            if let Some(location) = &related.location {
                writeln!(
                    output,
                    "  {}:{}:{}: {}",
                    location.path,
                    location.line + 1,
                    location.column + 1,
                    related.message
                )
                .map_err(CliError::Output)?;
            }
        }
    }
    if report.diagnostics.is_empty() {
        writeln!(output, "{command} passed ({})", report.target).map_err(CliError::Output)?;
    }
    Ok(())
}

pub(super) fn write_json<T: Serialize>(output: &mut impl Write, value: &T) -> Result<(), CliError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(CliError::Json)?;
    bytes.push(b'\n');
    output.write_all(&bytes).map_err(CliError::Output)?;
    Ok(())
}

pub(super) fn print_metadata(output: &mut impl Write, metadata: &Metadata) -> Result<(), CliError> {
    writeln!(
        output,
        "{} {} -> {} ({})",
        metadata.root.name,
        metadata.root.version.as_deref().unwrap_or(""),
        metadata.target,
        metadata.profile
    )
    .map_err(CliError::Output)?;
    writeln!(
        output,
        "  fill missing arguments: {}; PEX debug info: {}",
        metadata.fill_missing_arguments, metadata.debug_info
    )
    .map_err(CliError::Output)?;
    writeln!(
        output,
        "{} packages, {} dependencies, {} scripts",
        metadata.packages.len(),
        metadata.dependencies.len(),
        metadata.scripts.len()
    )
    .map_err(CliError::Output)?;
    for package in &metadata.packages {
        writeln!(
            output,
            "  {} {} [{}]",
            package.id.name,
            package.id.version.as_deref().unwrap_or(""),
            if package.id == metadata.root {
                "workspace"
            } else {
                "api"
            }
        )
        .map_err(CliError::Output)?;
    }
    Ok(())
}

pub(super) fn print_tree(output: &mut impl Write, metadata: &Metadata) -> Result<(), CliError> {
    writeln!(
        output,
        "{} {}",
        metadata.root.name,
        metadata.root.version.as_deref().unwrap_or("")
    )
    .map_err(CliError::Output)?;
    for edge in &metadata.dependencies {
        writeln!(
            output,
            "  {} -> {} {} ({:?})",
            edge.from.name, edge.to.name, edge.declared_path, edge.kind
        )
        .map_err(CliError::Output)?;
    }
    for script in &metadata.scripts {
        if script.providers.len() > 1 {
            let chain = script
                .providers
                .iter()
                .map(|provider| provider.package.name.as_str())
                .collect::<Vec<_>>()
                .join(" -> ");
            writeln!(output, "  script {}: {}", script.script, chain).map_err(CliError::Output)?;
        }
    }
    for requirement in &metadata.external_requirements {
        writeln!(
            output,
            "  external {}: {}",
            requirement.package.name, requirement.reason
        )
        .map_err(CliError::Output)?;
    }
    Ok(())
}

#[derive(Serialize)]
pub(super) struct TreeView<'a> {
    schema: u32,
    root: &'a PackageId,
    dependencies: &'a [DependencyEdge],
    scripts: &'a [ScriptSelection],
    external_requirements: &'a [ExternalRequirement],
}

impl<'a> From<&'a Metadata> for TreeView<'a> {
    fn from(metadata: &'a Metadata) -> Self {
        Self {
            schema: 3,
            root: &metadata.root,
            dependencies: &metadata.dependencies,
            scripts: &metadata.scripts,
            external_requirements: &metadata.external_requirements,
        }
    }
}
