//! Strict in-memory parsing of the single-package manifest format.

use std::{collections::BTreeMap, ops::Range};

use folio_project_model::{
    DependencyKind, DependencySpec, LocatedString, Manifest, SourceEncoding, SourceSpan,
};
use serde::Deserialize;
use toml::Spanned;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestErrorKind {
    Toml(String),
    InvalidValue { field: String, reason: &'static str },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestError {
    pub source: String,
    pub span: Option<Range<usize>>,
    pub kind: ManifestErrorKind,
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source)?;
        if let Some(span) = &self.span {
            write!(f, ":{}..{}", span.start, span.end)?;
        }
        match &self.kind {
            ManifestErrorKind::Toml(reason) => write!(f, ": {reason}"),
            ManifestErrorKind::InvalidValue { field, reason } => {
                write!(f, ": invalid {field}: {reason}")
            },
        }
    }
}

impl std::error::Error for ManifestError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    package: RawPackage,
    #[serde(default)]
    paths: RawPaths,
    languages: RawLanguages,
    build: RawBuild,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
    #[serde(default)]
    lint: RawLint,
    #[serde(default)]
    experimental: RawExperimental,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawPackage {
    name: Spanned<String>,
    version: Spanned<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPaths {
    source: Option<Spanned<String>>,
    output: Option<Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLanguages {
    papyrus: RawPapyrus,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPapyrus {
    dialect: Spanned<String>,
    extensions: Spanned<Vec<Spanned<String>>>,
    #[serde(default, rename = "user-flags")]
    user_flags: Vec<Spanned<RawUserFlag>>,
    #[serde(default, rename = "fill-missing-arguments")]
    fill_missing_arguments: Option<Spanned<bool>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawUserFlag {
    Name(String),
    Definition(folio_profiles::UserFlag),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBuild {
    target: Spanned<String>,
    profile: Spanned<String>,
    emit: Spanned<Vec<Spanned<String>>>,
    #[serde(default, rename = "debug-info")]
    debug_info: Option<Spanned<bool>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDependency {
    name: Spanned<String>,
    kind: Spanned<DependencyKind>,
    path: Spanned<String>,
    #[serde(default)]
    encoding: Option<Spanned<SourceEncoding>>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawExperimental {
    pex_dependencies: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLint {
    #[serde(default)]
    rules: BTreeMap<String, Spanned<String>>,
}

/// Parse a manifest from supplied text, preserving spans of every leaf field.
///
/// # Errors
/// Returns an error for malformed TOML, unknown fields, unsupported settings, or invalid manifest values.
pub fn parse(source: &str, input: &str) -> Result<Manifest, ManifestError> {
    let raw: RawManifest =
        toml::from_str(input).map_err(|error: toml::de::Error| ManifestError {
            source: source.to_owned(),
            span: error.span(),
            kind: ManifestErrorKind::Toml(error.to_string()),
        })?;

    let mut fields = BTreeMap::new();
    validate_core_fields(source, &raw, &mut fields)?;
    let source_path = path_setting(
        source,
        "paths.source",
        raw.paths.source,
        "Source/Scripts",
        &mut fields,
    )?;
    let output_path = path_setting(
        source,
        "paths.output",
        raw.paths.output,
        "Scripts",
        &mut fields,
    )?;
    if paths_overlap(&source_path.value, &output_path.value) {
        return Err(invalid(
            source,
            "paths.output",
            "source and output paths must not overlap",
            Some(output_path.span.start..output_path.span.end),
        ));
    }
    let extensions = parse_extensions(source, raw.languages.papyrus.extensions, &mut fields)?;
    let user_flags = parse_user_flags(source, raw.languages.papyrus.user_flags, &mut fields)?;
    let emit = parse_emit(source, raw.build.emit, &mut fields)?;
    let dependencies = parse_dependencies(source, raw.dependencies, &mut fields)?;
    let lint_rules = parse_lint_rules(source, raw.lint.rules, &mut fields)?;
    Ok(Manifest {
        source: source.to_owned(),
        fields,
        name: raw.package.name.into_inner(),
        version: raw.package.version.into_inner(),
        source_path,
        output_path,
        language: "papyrus".into(),
        dialect: raw.languages.papyrus.dialect.into_inner(),
        extensions,
        user_flags,
        fill_missing_arguments: raw
            .languages
            .papyrus
            .fill_missing_arguments
            .is_some_and(Spanned::into_inner),
        lint_rules,
        target: raw.build.target.into_inner(),
        profile: raw.build.profile.into_inner(),
        debug_info: raw.build.debug_info.is_none_or(Spanned::into_inner),
        experimental_pex_dependencies: raw.experimental.pex_dependencies,
        emit,
        dependencies,
    })
}

/// Validate supported language and build settings and retain their source locations.
fn validate_core_fields(
    source: &str,
    raw: &RawManifest,
    fields: &mut BTreeMap<String, SourceSpan>,
) -> Result<(), ManifestError> {
    for (field, value) in [
        ("package.name", &raw.package.name),
        ("package.version", &raw.package.version),
        ("languages.papyrus.dialect", &raw.languages.papyrus.dialect),
        ("build.target", &raw.build.target),
        ("build.profile", &raw.build.profile),
    ] {
        nonempty(source, field, value)?;
        fields.insert(field.into(), span(source, value.span()));
    }
    fields.insert(
        "languages.papyrus.extensions".into(),
        span(source, raw.languages.papyrus.extensions.span()),
    );
    fields.insert("build.emit".into(), span(source, raw.build.emit.span()));
    if let Some(value) = &raw.languages.papyrus.fill_missing_arguments {
        fields.insert(
            "languages.papyrus.fill-missing-arguments".into(),
            span(source, value.span()),
        );
    }
    if let Some(value) = &raw.build.debug_info {
        fields.insert("build.debug-info".into(), span(source, value.span()));
    }
    if raw.languages.papyrus.extensions.get_ref().is_empty() {
        return Err(invalid(
            source,
            "languages.papyrus.extensions",
            "must contain at least one extension",
            Some(raw.languages.papyrus.extensions.span()),
        ));
    }
    if raw.build.emit.get_ref().is_empty() {
        return Err(invalid(
            source,
            "build.emit",
            "must contain at least one output kind",
            Some(raw.build.emit.span()),
        ));
    }
    if raw.languages.papyrus.dialect.get_ref() != "skyrim" {
        return Err(invalid(
            source,
            "languages.papyrus.dialect",
            "only skyrim is supported",
            Some(raw.languages.papyrus.dialect.span()),
        ));
    }
    if raw.build.target.get_ref() != "skyrim-se" {
        return Err(invalid(
            source,
            "build.target",
            "only skyrim-se is supported",
            Some(raw.build.target.span()),
        ));
    }
    Ok(())
}

/// Parse and validate manifest extensions while preserving field spans.
fn parse_extensions(
    source: &str,
    values: Spanned<Vec<Spanned<String>>>,
    fields: &mut BTreeMap<String, SourceSpan>,
) -> Result<Vec<String>, ManifestError> {
    let extensions = values
        .into_inner()
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let field = format!("languages.papyrus.extensions[{index}]");
            nonempty(source, &field, &value)?;
            if value.get_ref() != "psc" {
                return Err(invalid(
                    source,
                    &field,
                    "only psc is supported",
                    Some(value.span()),
                ));
            }
            fields.insert(field, span(source, value.span()));
            Ok(value.into_inner())
        })
        .collect::<Result<Vec<_>, ManifestError>>()?;
    Ok(extensions)
}

/// Parse and validate manifest user flags while preserving field spans.
fn parse_user_flags(
    source: &str,
    values: Vec<Spanned<RawUserFlag>>,
    fields: &mut BTreeMap<String, SourceSpan>,
) -> Result<Vec<folio_profiles::UserFlag>, ManifestError> {
    let mut seen_flags = std::collections::BTreeSet::new();
    let user_flags = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let field = format!("languages.papyrus.user-flags[{index}]");
            let definition = match value.get_ref() {
                RawUserFlag::Name(name) => folio_profiles::UserFlag::from(name.clone()),
                RawUserFlag::Definition(definition) => definition.clone(),
            };
            let name = &definition.name;
            if !name
                .starts_with(|character: char| character.is_ascii_alphabetic() || character == '_')
                || !name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                return Err(invalid(
                    source,
                    &field,
                    "must be a Papyrus identifier",
                    Some(value.span()),
                ));
            }
            if name.eq_ignore_ascii_case("Hidden")
                || name.eq_ignore_ascii_case("Conditional")
                || folio_profiles::is_skyrim_keyword(name)
            {
                return Err(invalid(
                    source,
                    &field,
                    "built-in Papyrus flags must not be redeclared",
                    Some(value.span()),
                ));
            }
            if !seen_flags.insert(name.to_ascii_lowercase()) {
                return Err(invalid(
                    source,
                    &field,
                    "duplicate flag name",
                    Some(value.span()),
                ));
            }
            if folio_profiles::resolve_user_flags(std::slice::from_ref(&definition), 31).is_err() {
                return Err(invalid(
                    source,
                    &field,
                    "invalid flag bit or declaration scopes",
                    Some(value.span()),
                ));
            }
            fields.insert(field, span(source, value.span()));
            Ok(definition)
        })
        .collect::<Result<Vec<_>, ManifestError>>()?;
    folio_profiles::resolve_user_flags(&user_flags, 31).map_err(|_cause| {
        invalid(
            source,
            "languages.papyrus.user-flags",
            "flag names and bits must be unique and fit the target",
            None,
        )
    })?;
    Ok(user_flags)
}

/// Parse and validate manifest emit while preserving field spans.
fn parse_emit(
    source: &str,
    values: Spanned<Vec<Spanned<String>>>,
    fields: &mut BTreeMap<String, SourceSpan>,
) -> Result<Vec<String>, ManifestError> {
    let emit = values
        .into_inner()
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let field = format!("build.emit[{index}]");
            if value.get_ref() != "pex" {
                return Err(invalid(
                    source,
                    &field,
                    "only pex is supported",
                    Some(value.span()),
                ));
            }
            fields.insert(field, span(source, value.span()));
            Ok(value.into_inner())
        })
        .collect::<Result<Vec<_>, ManifestError>>()?;
    Ok(emit)
}

/// Parse and validate manifest dependencies while preserving field spans.
fn parse_dependencies(
    source: &str,
    values: Vec<RawDependency>,
    fields: &mut BTreeMap<String, SourceSpan>,
) -> Result<Vec<DependencySpec>, ManifestError> {
    let mut dependency_names = std::collections::BTreeSet::new();
    let dependencies = values
        .into_iter()
        .enumerate()
        .map(|(index, dep)| {
            let prefix = format!("dependencies[{index}]");
            nonempty(source, &format!("{prefix}.name"), &dep.name)?;
            relative(source, &format!("{prefix}.path"), &dep.path)?;
            if !dependency_names.insert(dep.name.get_ref().clone()) {
                return Err(invalid(
                    source,
                    &format!("{prefix}.name"),
                    "duplicate dependency alias",
                    Some(dep.name.span()),
                ));
            }
            if *dep.kind.get_ref() == DependencyKind::Repo {
                validate_repo_key(dep.path.get_ref()).map_err(|reason| {
                    invalid(
                        source,
                        &format!("{prefix}.path"),
                        reason,
                        Some(dep.path.span()),
                    )
                })?;
            }
            if dep.encoding.is_some() && *dep.kind.get_ref() != DependencyKind::Psc {
                return Err(invalid(
                    source,
                    &format!("{prefix}.encoding"),
                    "encoding is only valid for PSC dependencies",
                    dep.encoding.as_ref().map(Spanned::span),
                ));
            }
            if let Some(value) = &dep.encoding {
                fields.insert(format!("{prefix}.encoding"), span(source, value.span()));
            }
            for (field, range) in [
                ("name", dep.name.span()),
                ("kind", dep.kind.span()),
                ("path", dep.path.span()),
            ] {
                fields.insert(format!("{prefix}.{field}"), span(source, range));
            }
            Ok(DependencySpec {
                name: located(source, dep.name),
                kind: dep.kind.into_inner(),
                path: located(source, dep.path),
                encoding: dep
                    .encoding
                    .map_or(SourceEncoding::Utf8, Spanned::into_inner),
            })
        })
        .collect::<Result<Vec<_>, ManifestError>>()?;
    Ok(dependencies)
}

/// Parse and validate manifest lint rules while preserving field spans.
fn parse_lint_rules(
    source: &str,
    values: BTreeMap<String, Spanned<String>>,
    fields: &mut BTreeMap<String, SourceSpan>,
) -> Result<BTreeMap<String, String>, ManifestError> {
    let lint_rules = values
        .into_iter()
        .map(|(rule, level)| {
            let field = format!("lint.rules.{rule}");
            if !matches!(
                level.get_ref().as_str(),
                "off" | "info" | "warning" | "error"
            ) {
                return Err(invalid(
                    source,
                    &field,
                    "expected off, info, warning, or error",
                    Some(level.span()),
                ));
            }
            fields.insert(field, span(source, level.span()));
            Ok((rule, level.into_inner()))
        })
        .collect::<Result<BTreeMap<_, _>, ManifestError>>()?;
    Ok(lint_rules)
}

fn span(source: &str, range: Range<usize>) -> SourceSpan {
    SourceSpan {
        source: source.to_owned(),
        start: range.start,
        end: range.end,
    }
}

fn located(source: &str, value: Spanned<String>) -> LocatedString {
    let range = value.span();
    LocatedString {
        value: value.into_inner(),
        span: span(source, range),
    }
}

fn nonempty(source: &str, field: &str, value: &Spanned<String>) -> Result<(), ManifestError> {
    if value.get_ref().trim().is_empty() {
        Err(invalid(
            source,
            field,
            "must not be empty",
            Some(value.span()),
        ))
    } else {
        Ok(())
    }
}

/// Dependencies can reach a sibling project, but cannot use an absolute host path.
fn relative(source: &str, field: &str, value: &Spanned<String>) -> Result<(), ManifestError> {
    nonempty(source, field, value)?;
    let path = value.get_ref();
    if std::path::Path::new(path).is_absolute()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.get(1..2) == Some(":")
    {
        return Err(invalid(
            source,
            field,
            "must be a relative path",
            Some(value.span()),
        ));
    }
    Ok(())
}

/// Workspace paths stay below their manifest and use portable separators.
fn path_setting(
    source: &str,
    field: &str,
    value: Option<Spanned<String>>,
    default: &str,
    fields: &mut BTreeMap<String, SourceSpan>,
) -> Result<LocatedString, ManifestError> {
    let Some(value) = value else {
        return Ok(LocatedString {
            value: default.into(),
            span: span(source, 0..0),
        });
    };
    relative(source, field, &value)?;
    let path = value.get_ref();
    if path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path.eq_ignore_ascii_case(".folio")
        || path.to_ascii_lowercase().starts_with(".folio/")
    {
        return Err(invalid(
            source,
            field,
            "must be a workspace-relative path outside .folio",
            Some(value.span()),
        ));
    }
    fields.insert(field.into(), span(source, value.span()));
    Ok(located(source, value))
}

fn paths_overlap(source: &str, output: &str) -> bool {
    let source = source.to_ascii_lowercase();
    let output = output.to_ascii_lowercase();
    source == output
        || source.starts_with(&format!("{output}/"))
        || output.starts_with(&format!("{source}/"))
}

/// Repository keys use a portable, normalized path and never traverse parents.
///
/// # Errors
/// Returns an error if the key is empty, nonportable, reserved, or traverses parent directories.
pub fn validate_repo_key(key: &str) -> Result<(), &'static str> {
    if key.is_empty()
        || key.contains(['\\', ':', '*', '?', '<', '>', '|', '"'])
        || key.chars().any(char::is_control)
        || key.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || matches!(
                    part.split('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_uppercase()
                        .as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
        })
    {
        Err("must be a normalized repository key using / without parent traversal")
    } else {
        Ok(())
    }
}

fn invalid(
    source: &str,
    field: &str,
    reason: &'static str,
    span: Option<Range<usize>>,
) -> ManifestError {
    ManifestError {
        source: source.to_owned(),
        span,
        kind: ManifestErrorKind::InvalidValue {
            field: field.to_owned(),
            reason,
        },
    }
}

#[cfg(test)]
mod tests;
