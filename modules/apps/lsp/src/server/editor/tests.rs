use super::*;
use folio_analysis::AnalysisHost;
use folio_build::ProjectSource;
use folio_papyrus::PapyrusDialect;
use folio_source::Revision;

mod language_hover;

const SOURCE: &str = "ScriptName Demo\n{Demo documentation.}\nInt Function Add(Int left, Int right = 2)\n{Adds values.}\n Return left + right\nEndFunction\nInt Function Use()\n Return Add(1)\nEndFunction\n";

fn context() -> QueryContext {
    context_with(Vec::new())
}

fn context_with(external: Vec<folio_format_declarations::DeclarationBundle>) -> QueryContext {
    context_source(SOURCE, external)
}

fn context_source(
    source: &str,
    external: Vec<folio_format_declarations::DeclarationBundle>,
) -> QueryContext {
    let file = FileId(1);
    let path = PathBuf::from("D:/synthetic/Demo.psc");
    let uri = path_to_uri(&path);
    let mut host = AnalysisHost::new();
    host.set_external_declarations(external);
    host.upsert(file, Revision(1), Arc::from(source), PapyrusDialect::Skyrim)
        .unwrap();
    let view = ProjectAnalysisView {
        analysis: host.view(),
        sources: BTreeMap::from([(
            file,
            ProjectSource {
                package_key: "demo".into(),
                canonical_path: path.clone(),
                display_path: "Demo.psc".into(),
                script_candidate: "Demo".into(),
                dialect: PapyrusDialect::Skyrim,
                text: Arc::from(source),
            },
        )]),
        issues: Vec::new(),
    };
    QueryContext {
        view: Some(Arc::new(view)),
        metadata: None,
        loaded: None,
        paths: BTreeMap::from([(path, file)]),
        uris: BTreeMap::from([(uri, file)]),
        versions: BTreeMap::from([(file, Some(7))]),
        encoding: PositionEncoding::Utf16,
        generation: 4,
        settings: settings::EditorSettings::default(),
        markdown: true,
        commands: true,
        virtual_documents: true,
        overlays: BTreeMap::new(),
        completions: CompletionCache::default(),
    }
}

fn query(context: &QueryContext, byte: usize) -> Value {
    let position = folio_ide::position(SOURCE, byte, context.encoding).unwrap();
    json!({"textDocument":{"uri":context.uris.keys().next().unwrap()},
        "position":{"line":position.line,"character":position.character}})
}

#[test]
fn unsupported_documents_return_protocol_appropriate_empty_results() {
    assert_eq!(
        empty_answer("textDocument/semanticTokens/full"),
        json!({"data":[]})
    );
    assert_eq!(empty_answer("textDocument/codeLens"), json!([]));
    assert_eq!(empty_answer("textDocument/signatureHelp"), Value::Null);
}

#[test]
fn completion_resolution_preserves_the_candidate_and_adds_selected_documentation() {
    let context = context();
    let result = context
        .answer(
            "textDocument/completion",
            &query(&context, SOURCE.rfind("Add(1)").unwrap()),
        )
        .unwrap();
    let item = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == "Add")
        .unwrap();
    assert!(item.get("documentation").is_none());
    let resolved = context.answer("completionItem/resolve", item).unwrap();
    assert_eq!(resolved["textEdit"], item["textEdit"]);
    assert!(
        resolved["detail"]
            .as_str()
            .unwrap()
            .contains("Int Function Add")
    );
    assert!(
        resolved["documentation"]["value"]
            .as_str()
            .unwrap()
            .contains("Adds values.")
    );
    context.completions.clear();
    assert_eq!(
        context
            .answer("completionItem/resolve", item)
            .unwrap_err()
            .0,
        -32801
    );
}

#[test]
fn hover_has_owner_complete_declaration_documentation_and_navigation() {
    let context = context();
    let result = context
        .answer(
            "textDocument/hover",
            &query(&context, SOURCE.rfind("Add(1)").unwrap()),
        )
        .unwrap();
    let text = result["contents"]["value"].as_str().unwrap();
    assert!(text.starts_with("Demo\n"));
    let mut sections = text.split("\n\n---\n\n");
    let header = sections.next().unwrap();
    assert!(header.starts_with("Demo\n\n"));
    assert!(header.contains("Int Function Add(Int left, Int right = 2)"));
    assert!(sections.next().unwrap().contains("Adds values."));
    assert!(
        sections
            .next()
            .unwrap()
            .contains("command:folio.openLocation?")
    );
    let mut context = context;
    context.settings.documentation = false;
    let result = context
        .answer(
            "textDocument/hover",
            &query(&context, SOURCE.rfind("Add(1)").unwrap()),
        )
        .unwrap();
    assert!(
        !result["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Adds values.")
    );
    context.settings.details = false;
    let result = context
        .answer(
            "textDocument/hover",
            &query(&context, SOURCE.rfind("Add(1)").unwrap()),
        )
        .unwrap();
    assert!(
        !result["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("command:")
    );
}

#[test]
fn virtual_unknown_callable_navigation_uses_exact_generated_member_position() {
    let bundle=serde_json::from_value(json!({
        "format":"folio-declarations","schema":2,"profile":"papyrus-skyrim","origin":{"source":"fixture"},
        "scripts":[{"name":"External","documentation":"External API.","members":[{
            "name":"Run","documentation":"Runs a value.","kind":"unknown-callable","return_type":"Int",
            "parameters":[{"name":"value","ty":"Int","default":{"kind":"unknown"}}]
        }]}]
    })).unwrap();
    let context = context_with(vec![bundle]);
    let symbol = Symbol::Member {
        script: "External".into(),
        name: "Run".into(),
    };
    let location = context.declaration_location(&symbol).unwrap();
    assert_eq!(location["uri"], "folio-declaration:/External.psc");
    let params =
        json!({"textDocument":{"uri":location["uri"]},"position":location["range"]["start"]});
    let content = context
        .answer("folio/declarationContent", &json!({"uri":location["uri"]}))
        .unwrap();
    let text = content["text"].as_str().unwrap();
    let range = crate::protocol::parse_range(&location["range"]).unwrap();
    let start = folio_ide::offset(text, range.start, context.encoding).unwrap();
    let end = folio_ide::offset(text, range.end, context.encoding).unwrap();
    assert_eq!(&text[start..end], "Run");
    let result = context.answer("textDocument/hover", &params).unwrap();
    let hover = result["contents"]["value"].as_str().unwrap();
    assert!(hover.contains("Callable Run(Int value)"));
    assert!(hover.contains("Runs a value."));
    assert!(hover.contains("Defaults not recoverable from PEX"));
    assert!(!hover.contains("Function Run"));
}

#[test]
fn reference_lens_resolves_to_the_real_call() {
    let context = context();
    let uri = context.uris.keys().next().unwrap();
    let lenses = context
        .answer(
            "textDocument/codeLens",
            &json!({"textDocument":{"uri":uri}}),
        )
        .unwrap();
    let target = lenses
        .as_array()
        .unwrap()
        .iter()
        .find(|lens| lens["data"]["kind"] == "references" && lens["range"]["start"]["line"] == 2)
        .unwrap();
    let result = context.answer("codeLens/resolve", target).unwrap();
    let locations = result["command"]["arguments"][2].as_array().unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(
        locations[0]["range"]["start"],
        json!({"line":7,"character":8})
    );
    let mut stale = target.clone();
    stale["data"]["generation"] = json!(3);
    assert_eq!(
        context.answer("codeLens/resolve", &stale).unwrap_err().0,
        -32801
    );
}

#[test]
fn implementation_lenses_apply_to_scripts_and_instance_callables() {
    let source = "ScriptName Demo\nInt Property Amount Auto\nFunction Utility() Global\nEndFunction\nFunction Run()\nEndFunction\n";
    let context = context_source(source, Vec::new());
    let uri = context.uris.keys().next().unwrap();
    let lenses = context
        .answer(
            "textDocument/codeLens",
            &json!({"textDocument":{"uri":uri}}),
        )
        .unwrap();
    let hierarchy_lines = lenses
        .as_array()
        .unwrap()
        .iter()
        .filter(|lens| lens["data"]["kind"] == "implementations")
        .map(|lens| lens["range"]["start"]["line"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(hierarchy_lines, vec![0, 4]);
    for line in [1, 2] {
        assert!(lenses.as_array().unwrap().iter().any(|lens| {
            lens["data"]["kind"] == "references" && lens["range"]["start"]["line"] == line
        }));
    }
}

#[test]
fn rename_returns_versioned_edits_for_declaration_and_use() {
    let context = context();
    let mut params = query(&context, SOURCE.find("left,").unwrap());
    params["newName"] = json!("first");
    let result = context.answer("textDocument/rename", &params).unwrap();
    let documents = result["documentChanges"].as_array().unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0]["textDocument"]["version"], 7);
    let edits = documents[0]["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| edit["newText"] == "first"));
    assert!(
        edits
            .iter()
            .any(|edit| edit["range"]["start"] == json!({"line":2,"character":21}))
    );
    assert!(
        edits
            .iter()
            .any(|edit| edit["range"]["start"] == json!({"line":4,"character":8}))
    );
}
