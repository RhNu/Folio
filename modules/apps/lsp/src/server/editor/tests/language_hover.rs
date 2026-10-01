use super::*;
use folio_declaration_tools::{GenerationOptions, SourceInput, generate};
use folio_project_model::{
    DeclarationLocation, DeclaredScript, DependencyKind, LoadedDependency, LoadedRoot, SourceId,
};
use folio_project_resolve::io::{FolioHome, InputSnapshot, WatchPlan};

fn params(context: &QueryContext, uri: &str, text: &str, needle: &str) -> Value {
    let byte = text
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
        .unwrap_or_else(|| panic!("missing {needle} in {text}"));
    let position = folio_ide::position(text, byte, context.encoding).unwrap();
    json!({"textDocument":{"uri":uri}, "position":{"line":position.line,"character":position.character}})
}

fn value(context: &QueryContext, uri: &str, text: &str, needle: &str) -> Value {
    context
        .answer("textDocument/hover", &params(context, uri, text, needle))
        .unwrap()
}

fn dependency(source: &str) -> QueryContext {
    let bundle = generate(
        GenerationOptions {
            source: "synthetic API",
        },
        &[SourceInput {
            path: "External.psc",
            text: source,
        }],
    )
    .unwrap();
    let mut context = context_with(vec![bundle.clone()]);
    let manifest = folio_project_resolve::manifest::parse("folio.toml",
        "[package]\nname='demo'\nversion='0.1.0'\n[languages.papyrus]\ndialect='skyrim'\nextensions=['psc']\n[build]\ntarget='skyrim-se'\nprofile='dev'\nemit=['pex']\n[[dependencies]]\nname='api'\nkind='psc'\npath='../api'\n").unwrap();
    let dependency = LoadedDependency {
        source_key: "dependency:0".into(),
        source_id: SourceId::Dependency {
            index: 0,
            kind: DependencyKind::Psc,
            path: "../api".into(),
            digest: folio_format_declarations::semantic_digest(&bundle),
        },
        kind: DependencyKind::Psc,
        name: "api".into(),
        declared_path: "../api".into(),
        canonical_path: PathBuf::from("D:/synthetic/api"),
        declaration: manifest.dependencies[0].path.span.clone(),
        profile: bundle.profile.clone(),
        scripts: vec![DeclaredScript {
            name: "External".into(),
            location: DeclarationLocation {
                carrier_path: "../api".into(),
                script_name: "External".into(),
                source_path: Some("External.psc".into()),
                line: Some(1),
                column: Some(1),
            },
        }],
    };
    let root = LoadedRoot {
        source_key: "root".into(),
        source_id: SourceId::Project,
        manifest,
        source_files: Vec::new(),
    };
    let metadata =
        folio_project_resolve::resolve(&root, std::slice::from_ref(&dependency)).unwrap();
    context.loaded = Some(LoadedProject {
        root_key: "root".into(),
        root,
        dependencies: vec![dependency],
        declaration_bundles: BTreeMap::from([("dependency:0".into(), bundle)]),
        source_inputs: Vec::new(),
        input_snapshots: vec![InputSnapshot::File {
            path: PathBuf::from("D:/synthetic/api/External.psc"),
            bytes: Arc::from(source.as_bytes()),
        }],
        watch_plan: WatchPlan::default(),
        folio_home: FolioHome {
            path: PathBuf::from("D:/synthetic/home"),
        },
    });
    context.metadata = Some(Arc::new(metadata));
    context
}

#[test]
fn language_help_renders_examples_references_and_exact_encoded_ranges() {
    let source = "ScriptName Demo\r\nFunction Run()\r\n String label = \"雪🦊\"\r\n If True\r\n EndIf\r\nEndFunction\r\n";
    let mut context = context_source(source, Vec::new());
    let uri = context.uris.keys().next().unwrap().clone();
    let result = value(&context, &uri, source, "If True");
    let hover = result["contents"]["value"].as_str().unwrap();
    assert!(hover.starts_with("Skyrim Papyrus\n\n~~~papyrus\nIf\n~~~"));
    assert!(hover.contains("~~~papyrus\nIf count > 0"));
    assert!(hover.contains(
        "[Creation Kit reference (Skyrim)](https://ck.uesp.net/wiki/Statement_Reference)"
    ));
    assert_eq!(
        result["range"],
        json!({"start":{"line":3,"character":1},"end":{"line":3,"character":3}})
    );
    for (encoding, end) in [(PositionEncoding::Utf16, 21), (PositionEncoding::Utf8, 25)] {
        context.encoding = encoding;
        let result = value(&context, &uri, source, "\"雪");
        assert_eq!(result["range"]["start"]["character"], 16);
        assert_eq!(result["range"]["end"]["character"], end);
    }
}

#[test]
fn documentation_and_details_settings_apply_independently_and_plaintext_has_urls() {
    let source = "ScriptName Demo\nInt Property Count = 42 Auto\n";
    let mut context = context_source(source, Vec::new());
    let uri = context.uris.keys().next().unwrap().clone();
    context.settings.details = false;
    let result = value(&context, &uri, source, "Int");
    let hover = result["contents"]["value"].as_str().unwrap();
    assert!(hover.contains("32-bit"));
    assert!(hover.contains("Creation Kit reference"));
    assert!(!hover.contains("Skyrim Papyrus"));
    context.settings.details = true;
    context.settings.documentation = false;
    let result = value(&context, &uri, source, "42");
    let hover = result["contents"]["value"].as_str().unwrap();
    assert!(hover.contains("Value of literal: 42"));
    assert!(!hover.contains("32-bit"));
    assert!(!hover.contains("Creation Kit reference"));
    context.settings.documentation = true;
    context.markdown = false;
    let result = value(&context, &uri, source, "Auto");
    assert_eq!(result["contents"]["kind"], "plaintext");
    let hover = result["contents"]["value"].as_str().unwrap();
    assert!(hover.contains("Int Property Count = 0 Auto"));
    assert!(
        hover.contains(
            "Creation Kit reference (Skyrim): https://ck.uesp.net/wiki/Property_Reference"
        )
    );
    assert!(!hover.contains("~~~"));
}

#[test]
fn string_values_cannot_inject_trusted_markdown_actions() {
    let source = "ScriptName Demo\nString Property Text = \"[run](command:evil) <img>\" Auto\n";
    let context = context_source(source, Vec::new());
    let uri = context.uris.keys().next().unwrap();
    let result = value(&context, uri, source, "\"[run]");
    let hover = result["contents"]["value"].as_str().unwrap();
    assert!(hover.contains(r"\[run\]\(command:evil\)"));
    assert!(hover.contains("&lt;img&gt;"));
    assert!(!hover.contains("[run](command:evil)"));
    assert!(hover.contains("[Creation Kit reference (Skyrim)]("));
}

#[test]
fn selected_psc_and_virtual_declarations_share_language_help_and_preserve_member_docs() {
    let source = "ScriptName External\nInt Property Count = 42 Auto\n{Counter documentation.}\nString Function Run(String label = \"hello\") Native\n";
    let mut context = dependency(source);
    let psc_uri = path_to_uri(&PathBuf::from("D:/synthetic/api/External.psc"));
    let virtual_uri = presentation::virtual_uri("External");
    let virtual_text = context.document_source(&virtual_uri).unwrap();
    for (uri, text) in [(&psc_uri, source), (&virtual_uri, virtual_text.as_str())] {
        for needle in ["ScriptName", "Int", "42", "Auto", "Native"] {
            let result = value(&context, uri, text, needle);
            assert_ne!(result, Value::Null, "{uri}: {needle}");
            let hover = result["contents"]["value"].as_str().unwrap();
            assert!(
                hover.contains("Creation Kit reference (Skyrim)"),
                "{needle}: {hover}"
            );
            assert!(!hover.contains("command:"));
        }
        let result = value(&context, uri, text, "Count");
        let hover = result["contents"]["value"].as_str().unwrap();
        assert!(hover.contains("Counter documentation."));
        assert!(!hover.contains("Creation Kit reference"));
    }
    let changed = source.replace("42", "7");
    context.overlays.insert(
        PathBuf::from("D:/synthetic/api/External.psc"),
        Arc::from(changed.as_str()),
    );
    let result = value(&context, &psc_uri, &changed, "7");
    assert!(
        result["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Value of literal: 7")
    );
}

#[test]
fn unsupported_document_and_comment_locations_have_no_hover() {
    let context = context();
    let result = context.answer("textDocument/hover", &json!({"textDocument":{"uri":"folio-declaration:/Missing.psc"},"position":{"line":0,"character":0}})).unwrap();
    assert_eq!(result, Value::Null);
    let source = "ScriptName Demo\nFunction Run()\n ; If Int\n Int value = 2 + 3\nEndFunction\n";
    let context = context_source(source, Vec::new());
    let uri = context.uris.keys().next().unwrap();
    assert_eq!(value(&context, uri, source, "If Int"), Value::Null);
    assert_eq!(value(&context, uri, source, " +"), Value::Null);
}
