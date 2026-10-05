use folio_project_model::{PackageId, SourceId};

use super::*;
use crate::plan::{BuildUnit, PlannedScript};

fn fixture() -> (ProjectPlan, UnitRecord) {
    let package = PackageId {
        name: "app".into(),
        version: Some("1".into()),
        source: SourceId::Project,
    };
    let script = PlannedScript {
        script: "sky".into(),
        source_path: "src/Sky.psc".into(),
        package: package.clone(),
        artifact_path: "sky.pex".into(),
    };
    let plan = ProjectPlan {
        schema: 3,
        root: package.clone(),
        output: "Scripts".into(),
        units: vec![BuildUnit {
            id: "abc".into(),
            package,
            target: "skyrim-se".into(),
            profile: "dev".into(),
            scripts: vec![script],
        }],
        external_requirements: vec![],
    };
    let record = UnitRecord {
        unit_id: "abc".into(),
        package: "app".into(),
        package_version: "1".into(),
        package_source: serde_json::json!({"source_kind":"project"}),
        target: "skyrim-se".into(),
        profile: "dev".into(),
        fingerprint: "key".into(),
        generation: "abc/generations/g123".into(),
        artifacts: vec![ArtifactRecord {
            kind: "pex".into(),
            path: "sky.pex".into(),
            digest: "digest".into(),
            format: "pex-3.2-skyrim-se".into(),
            script: "sky".into(),
            source_package: "app".into(),
            source_path: "src/Sky.psc".into(),
        }],
    };
    (plan, record)
}

#[test]
fn publication_requires_exact_completed_selection_and_no_cancellation() {
    let (plan, unit) = fixture();
    assert!(matches!(
        prepare_success(&plan, vec![], false),
        Err(BuildError::InvalidSelection(_))
    ));
    assert!(matches!(
        prepare_success(&plan, vec![unit.clone()], true),
        Err(BuildError::Cancelled)
    ));
    let mut wrong = unit.clone();
    wrong.artifacts[0].source_path = "src/Other.psc".into();
    assert!(matches!(
        prepare_success(&plan, vec![wrong], false),
        Err(BuildError::InvalidSelection(_))
    ));
    let complete = prepare_success(&plan, vec![unit.clone()], false).unwrap();
    assert_eq!(complete.units, vec![unit]);
}

#[test]
fn reused_artifacts_record_current_dependency_provenance() {
    let (mut plan, unit) = fixture();
    let current_source = SourceId::Dependency {
        index: 0,
        kind: folio_project_model::DependencyKind::Decl,
        path: "new/api.fdecl".into(),
        digest: "semantic-api".into(),
    };
    plan.external_requirements
        .push(folio_project_model::ExternalRequirement {
            package: PackageId {
                name: "api".into(),
                version: None,
                source: SourceId::Dependency {
                    index: 0,
                    kind: folio_project_model::DependencyKind::Decl,
                    path: "old/api.json".into(),
                    digest: "semantic-api".into(),
                },
            },
            target: "skyrim-se".into(),
            abi: "papyrus-skyrim".into(),
            reason: "runtime API".into(),
        });
    let previous = prepare_success(&plan, vec![unit], false).unwrap();
    plan.external_requirements[0].package.source = current_source.clone();
    let refreshed = prepare_success(&plan, previous.units.clone(), false).unwrap();
    assert_eq!(refreshed.units, previous.units);
    assert_ne!(
        refreshed.external_requirements,
        previous.external_requirements
    );
    assert_eq!(
        refreshed.external_requirements[0].package_source,
        serde_json::to_value(current_source).unwrap()
    );
    assert_eq!(refreshed.external_requirements[0].package_version, None);
}
