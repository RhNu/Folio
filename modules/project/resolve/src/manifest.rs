//! Strict in-memory parsing of the single-package manifest format.

use std::{collections::BTreeMap, ops::Range};

use folio_project_model::{DependencyKind, DependencySpec, LocatedString, Manifest, SourceSpan};
use serde::Deserialize;
use toml::Spanned;

pub const SCHEMA_VERSION: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestErrorKind {
    Toml(String),
    UnsupportedSchema(u32),
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
            ManifestErrorKind::UnsupportedSchema(value) => {
                write!(f, ": unsupported manifest schema {value}")
            }
            ManifestErrorKind::InvalidValue { field, reason } => {
                write!(f, ": invalid {field}: {reason}")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    schema: Spanned<u32>,
    package: RawPackage,
    #[serde(default)]
    paths: RawPaths,
    languages: RawLanguages,
    build: RawBuild,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
    #[serde(default)]
    lint: RawLint,
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
    user_flags: Vec<Spanned<String>>,
    #[serde(default, rename = "fill-missing-arguments")]
    fill_missing_arguments: Option<Spanned<bool>>,
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
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLint {
    #[serde(default)]
    rules: BTreeMap<String, Spanned<String>>,
}

/// Parse a manifest from supplied text, preserving spans of every leaf field.
pub fn parse(source: &str, input: &str) -> Result<Manifest, ManifestError> {
    let raw: RawManifest =
        toml::from_str(input).map_err(|error: toml::de::Error| ManifestError {
            source: source.to_owned(),
            span: error.span(),
            kind: ManifestErrorKind::Toml(error.to_string()),
        })?;
    if *raw.schema.get_ref() != SCHEMA_VERSION {
        return Err(ManifestError {
            source: source.to_owned(),
            span: Some(raw.schema.span()),
            kind: ManifestErrorKind::UnsupportedSchema(*raw.schema.get_ref()),
        });
    }

    let mut fields = BTreeMap::new();
    fields.insert("schema".into(), span(source, raw.schema.span()));
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
    let extensions = raw
        .languages
        .papyrus
        .extensions
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
    let mut seen_flags = std::collections::BTreeSet::new();
    let user_flags = raw
        .languages
        .papyrus
        .user_flags
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let field = format!("languages.papyrus.user-flags[{index}]");
            let name = value.get_ref();
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
            if name.eq_ignore_ascii_case("Hidden") || name.eq_ignore_ascii_case("Conditional") {
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
            fields.insert(field, span(source, value.span()));
            Ok(value.into_inner())
        })
        .collect::<Result<Vec<_>, ManifestError>>()?;
    let emit = raw
        .build
        .emit
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
    let dependencies = raw
        .dependencies
        .into_iter()
        .enumerate()
        .map(|(index, dep)| {
            let prefix = format!("dependencies[{index}]");
            nonempty(source, &format!("{prefix}.name"), &dep.name)?;
            relative(source, &format!("{prefix}.path"), &dep.path)?;
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
            })
        })
        .collect::<Result<Vec<_>, ManifestError>>()?;
    let lint_rules = raw
        .lint
        .rules
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
            .is_some_and(|value| value.into_inner()),
        lint_rules,
        target: raw.build.target.into_inner(),
        profile: raw.build.profile.into_inner(),
        debug_info: raw.build.debug_info.is_none_or(|value| value.into_inner()),
        emit,
        dependencies,
    })
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
mod tests {
    use super::*;

    const MINIMAL: &str = r#"schema = 3
[package]
name = "mod-a"
version = "0.1.0"
[languages.papyrus]
dialect = "skyrim"
extensions = ["psc"]
[build]
target = "skyrim-se"
profile = "dev"
emit = ["pex"]
"#;

    #[test]
    fn parses_minimal_and_preserves_field_offsets() {
        let manifest = parse("folio.toml", MINIMAL).unwrap();
        assert_eq!(manifest.name, "mod-a");
        assert_eq!(manifest.source_path.value, "Source/Scripts");
        assert_eq!(manifest.output_path.value, "Scripts");
        assert!(!manifest.fill_missing_arguments);
        assert!(manifest.debug_info);
        let span = &manifest.fields["package.name"];
        assert_eq!(&MINIMAL[span.start..span.end], "\"mod-a\"");
    }

    #[test]
    fn parses_call_and_debug_policies_with_field_positions() {
        let input = MINIMAL
            .replace(
                "extensions = [\"psc\"]",
                "extensions = [\"psc\"]\nfill-missing-arguments = true",
            )
            .replace("emit = [\"pex\"]", "emit = [\"pex\"]\ndebug-info = false");
        let manifest = parse("folio.toml", &input).unwrap();
        assert!(manifest.fill_missing_arguments);
        assert!(!manifest.debug_info);
        for field in [
            "languages.papyrus.fill-missing-arguments",
            "build.debug-info",
        ] {
            let location = &manifest.fields[field];
            assert!(matches!(
                &input[location.start..location.end],
                "true" | "false"
            ));
        }
    }

    #[test]
    fn accepts_explicit_builtin_dependency_identity() {
        let input = format!(
            "{MINIMAL}\n[[dependencies]]\nname = \"ck-1.6.1170\"\nkind = \"builtin\"\npath = \"ck-1.6.1170\"\n"
        );
        let manifest = parse("folio.toml", &input).unwrap();
        assert_eq!(manifest.dependencies[0].kind, DependencyKind::Builtin);
        assert_eq!(manifest.dependencies[0].path.value, "ck-1.6.1170");
    }

    #[test]
    fn keeps_declared_dependency_precedence() {
        let input = format!(
            "{MINIMAL}\n[[dependencies]]\nname = \"ck\"\nkind = \"builtin\"\npath = \"ck\"\n[[dependencies]]\nname = \"skse\"\nkind = \"builtin\"\npath = \"skse\"\n"
        );
        let manifest = parse("folio.toml", &input).unwrap();
        assert_eq!(
            manifest
                .dependencies
                .iter()
                .map(|item| item.name.value.as_str())
                .collect::<Vec<_>>(),
            vec!["ck", "skse"]
        );
    }

    #[test]
    fn rejects_unknown_field_with_position() {
        let input = MINIMAL.replace("profile = \"dev\"", "profiel = \"dev\"");
        let error = parse("folio.toml", &input).unwrap_err();
        assert!(matches!(error.kind, ManifestErrorKind::Toml(_)));
        assert!(error.span.is_some());
    }

    #[test]
    fn paths_must_be_distinct_workspace_directories() {
        let input = format!(
            "{MINIMAL}\n[paths]\nsource = \"Source/Scripts\"\noutput = \"Source/Scripts/build\"\n"
        );
        let error = parse("folio.toml", &input).unwrap_err();
        assert!(matches!(error.kind, ManifestErrorKind::InvalidValue { .. }));
    }

    #[test]
    fn custom_source_and_output_are_read_from_one_workspace() {
        let input = format!("{MINIMAL}\n[paths]\nsource = \"Papyrus\"\noutput = \"Build/Pex\"\n");
        let manifest = parse("folio.toml", &input).unwrap();
        assert_eq!(manifest.source_path.value, "Papyrus");
        assert_eq!(manifest.output_path.value, "Build/Pex");
        let field = &manifest.fields["paths.output"];
        assert_eq!(&input[field.start..field.end], "\"Build/Pex\"");
    }

    #[test]
    fn validates_explicit_papyrus_user_flags() {
        let input = MINIMAL.replace(
            "extensions = [\"psc\"]",
            "extensions = [\"psc\"]\nuser-flags = [\"MyFlag\"]",
        );
        let manifest = parse("folio.toml", &input).unwrap();
        assert_eq!(manifest.user_flags, vec!["MyFlag"]);
        let duplicated = input.replace("[\"MyFlag\"]", "[\"MyFlag\", \"myflag\"]");
        assert!(matches!(
            parse("folio.toml", &duplicated),
            Err(ManifestError {
                kind: ManifestErrorKind::InvalidValue { .. },
                ..
            })
        ));
        let invalid = input.replace("MyFlag", "1Flag");
        assert!(matches!(
            parse("folio.toml", &invalid),
            Err(ManifestError {
                kind: ManifestErrorKind::InvalidValue { .. },
                ..
            })
        ));
    }

    #[test]
    fn built_in_flags_cannot_be_redeclared() {
        let input = MINIMAL.replace(
            "extensions = [\"psc\"]",
            "extensions = [\"psc\"]\nuser-flags = [\"hIdDeN\"]",
        );
        let error = parse("folio.toml", &input).unwrap_err();
        assert!(matches!(error.kind, ManifestErrorKind::InvalidValue { .. }));
        let range = error.span.expect("flag has source location");
        assert_eq!(&input[range], "\"hIdDeN\"");
    }

    #[test]
    fn lint_levels_are_parsed_and_invalid_levels_are_located() {
        let input =
            format!("{MINIMAL}\n[lint.rules]\n\"papyrus.prefer-truthy-none-check\" = \"error\"\n");
        let manifest = parse("folio.toml", &input).unwrap();
        assert_eq!(
            manifest.lint_rules["papyrus.prefer-truthy-none-check"],
            "error"
        );

        let invalid_input = input.replace("= \"error\"", "= \"fatal\"");
        let error = parse("folio.toml", &invalid_input).unwrap_err();
        assert!(matches!(error.kind, ManifestErrorKind::InvalidValue { .. }));
        let range = error.span.expect("lint level has source location");
        assert_eq!(&invalid_input[range], "\"fatal\"");
    }
}
