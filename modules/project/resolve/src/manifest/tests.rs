use super::*;

const MINIMAL: &str = r#"[package]
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
fn pex_dependency_gate_is_explicit() {
    let dependency =
        "\n[[dependencies]]\nname = \"binary\"\nkind = \"pex\"\npath = \"../Binary/Scripts\"\n";
    let disabled = parse("folio.toml", &format!("{MINIMAL}{dependency}")).unwrap();
    assert!(!disabled.experimental_pex_dependencies);
    assert_eq!(disabled.dependencies[0].kind, DependencyKind::Pex);
    let enabled = parse(
        "folio.toml",
        &format!("{MINIMAL}\n[experimental]\npex-dependencies = true\n{dependency}"),
    )
    .unwrap();
    assert!(enabled.experimental_pex_dependencies);
}

#[test]
fn keeps_declared_dependency_precedence() {
    let input = format!(
        "{MINIMAL}\n[[dependencies]]\nname = \"ck\"\nkind = \"repo\"\npath = \"ck\"\n[[dependencies]]\nname = \"skse\"\nkind = \"repo\"\npath = \"skse\"\n"
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
fn aliases_are_local_and_repeated_carriers_keep_their_occurrences() {
    let dependency = "\n[[dependencies]]\nname = \"first\"\nkind = \"decl\"\npath = \"api.json\"\n";
    let input = format!(
        "{MINIMAL}{dependency}{}",
        dependency.replace("first", "second")
    );
    let manifest = parse("folio.toml", &input).unwrap();
    assert_eq!(manifest.dependencies.len(), 2);
    let duplicate = format!("{MINIMAL}{dependency}{dependency}");
    assert!(matches!(
        parse("folio.toml", &duplicate).unwrap_err().kind,
        ManifestErrorKind::InvalidValue { .. }
    ));
}

#[test]
fn psc_encoding_is_explicit_and_located() {
    let input = format!(
        "{MINIMAL}\n[[dependencies]]\nname = \"api\"\nkind = \"psc\"\npath = \"../scripts\"\nencoding = \"windows1252\"\n"
    );
    let manifest = parse("folio.toml", &input).unwrap();
    assert_eq!(
        manifest.dependencies[0].encoding,
        SourceEncoding::Windows1252
    );
    let location = &manifest.fields["dependencies[0].encoding"];
    assert_eq!(&input[location.start..location.end], "\"windows1252\"");
    let invalid_input = input.replace("kind = \"psc\"", "kind = \"decl\"");
    assert!(parse("folio.toml", &invalid_input).is_err());
}

#[test]
fn repo_keys_are_portable_without_parent_traversal() {
    for key in ["ck/1.6.1170.0", "skse/2.2.8.json"] {
        assert!(validate_repo_key(key).is_ok());
    }
    for key in [
        "../api",
        "/api",
        "api//v1",
        "api/../v1",
        "api\\v1",
        "C:/api",
        "con/file",
        "api/v1.",
    ] {
        assert!(validate_repo_key(key).is_err(), "{key}");
    }
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
    assert_eq!(
        manifest.user_flags,
        vec![folio_profiles::UserFlag::from("MyFlag")]
    );
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
fn typed_flags_preserve_explicit_bits_scopes_and_reject_conflicts() {
    let input = MINIMAL.replace(
        "extensions = [\"psc\"]",
        "extensions = [\"psc\"]\nuser-flags = [\"Legacy\", { name = \"ApiTag\", bit = 7, scopes = [\"script\", \"function\"] }]",
    );
    let manifest = parse("folio.toml", &input).unwrap();
    assert_eq!(manifest.user_flags[1].bit, Some(7));
    assert_eq!(
        manifest.user_flags[1].scopes,
        [
            folio_profiles::FlagScope::Script,
            folio_profiles::FlagScope::Function
        ]
    );
    for invalid in [
        input.replace("bit = 7", "bit = 1"),
        input.replace("\"script\", \"function\"", "\"state\""),
        input.replace("\"Legacy\"", "{ name = \"Legacy\", bit = 7 }"),
        input.replace("name = \"ApiTag\"", "name = \"Native\""),
    ] {
        assert!(parse("folio.toml", &invalid).is_err());
    }
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
