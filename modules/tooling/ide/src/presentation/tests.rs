use super::*;
use crate::tests::{file, project};
use folio_analysis::AnalysisHost;
use folio_format_declarations::{
    DeclarationBundle, Member, MemberData, Origin, Parameter, ParameterDefault, Script,
};

fn external() -> ProjectAnalysisView {
    let member = Member {
        name: "Work".into(),
        documentation: Some("Performs work.".into()),
        flags: vec!["Global".into(), "Hidden".into()],
        data: MemberData::UnknownCallable {
            return_type: Some("Int".into()),
            global: true,
            native: false,
            parameters: vec![Parameter {
                name: "amount".into(),
                ty: "Int".into(),
                default: ParameterDefault::Unknown,
            }],
        },
    };
    let bundle = DeclarationBundle {
        format: folio_format_declarations::FORMAT.into(),
        schema: folio_format_declarations::SCHEMA_VERSION,
        profile: folio_format_declarations::PROFILE.into(),
        origin: Origin {
            source: "Synthetic API".into(),
            input_digest: None,
        },
        scripts: vec![Script {
            name: "Api".into(),
            documentation: Some("API documentation.".into()),
            parent: None,
            is_native: false,
            flags: Vec::new(),
            imports: Vec::new(),
            members: vec![member],
            states: Vec::new(),
            source: None,
        }],
    };
    let mut host = AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    ProjectAnalysisView {
        analysis: host.view(),
        sources: Default::default(),
        issues: Vec::new(),
    }
}

#[test]
fn source_hover_preserves_defaults_flags_docs_and_excludes_body() {
    let text = "Scriptname Example\nInt Function Work(Int amount = 2)\n{Does useful work.}\n Return amount\nEndFunction\nFunction Use()\n Int value = Work()\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let hover = crate::hover(&view, file, text.rfind("Work").unwrap()).unwrap();
    assert!(hover.declaration.contains("amount = 2"));
    assert!(!hover.declaration.contains("Return"));
    assert_eq!(hover.documentation.as_deref(), Some("Does useful work."));
    assert!(hover.span.is_some());
}

#[test]
fn external_hover_never_claims_source_location_or_unknown_callable_kind() {
    let view = external();
    let hover = hover_symbol(
        &view,
        &Symbol::Member {
            script: "Api".into(),
            name: "Work".into(),
        },
    )
    .unwrap();
    assert!(hover.span.is_none());
    assert!(hover.declaration.contains("Callable Work"));
    assert!(!hover.declaration.contains("Function"));
    assert_eq!(hover.documentation.as_deref(), Some("Performs work."));
    assert_eq!(hover.details.len(), 2);
    assert_eq!(hover.declaration.matches("Global").count(), 1);
}

#[test]
fn external_manual_property_hover_includes_known_accessor_shapes() {
    let mut view = external();
    let mut bundle = view.analysis.external_declarations()[0].clone();
    bundle.scripts[0].members.push(Member {
        name: "Count".into(),
        documentation: None,
        flags: Vec::new(),
        data: MemberData::Property {
            ty: "Int".into(),
            access: folio_format_declarations::PropertyAccess::Manual {
                readable: true,
                writable: true,
            },
            initial_literal: None,
        },
    });
    let mut host = AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    view.analysis = host.view();
    let hover = hover_symbol(
        &view,
        &Symbol::Member {
            script: "Api".into(),
            name: "Count".into(),
        },
    )
    .unwrap();
    assert!(hover.declaration.contains("Int Function Get()"));
    assert!(hover.declaration.contains("Function Set(Int value)"));
    assert!(hover.declaration.ends_with("EndProperty"));
}

#[test]
fn virtual_declarations_retain_exact_name_ranges_for_unknown_callable() {
    let view = external();
    let document = declaration_document(&view, "Api").unwrap();
    assert_eq!(document.declarations.len(), 2);
    for (symbol, range) in &document.declarations {
        assert_eq!(
            &document.text[range.start..range.end],
            crate::navigation::name(symbol)
        );
    }
    assert!(document.text.contains("; Int Callable Work"));
    assert!(document.text.contains("Default for amount is unknown"));
    assert!(!document.text.contains("EndFunction"));
}

#[test]
fn state_hover_includes_documentation() {
    let text = "Scriptname Example\nState Busy\n{Busy state.}\nEndState\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let hover = crate::hover(&view, file, text.find("Busy").unwrap()).unwrap();
    assert_eq!(hover.documentation.as_deref(), Some("Busy state."));
    assert_eq!(hover.declaration, "State Busy");
}
