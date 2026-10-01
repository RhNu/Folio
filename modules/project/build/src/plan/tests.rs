use super::*;
use crate::tests::support as test_support;

#[test]
fn target_flag_capacity_reserves_builtin_bits() {
    let loaded = test_support::fixture("Scriptname Sky", "Sky");
    let mut metadata = test_support::resolve(&loaded);
    metadata.user_flags = (0..30).map(|index| format!("Flag{index}")).collect();
    assert!(project_plan(&loaded, &metadata).is_ok());
    metadata.user_flags.push("OneMore".into());
    assert_eq!(
        project_plan(&loaded, &metadata),
        Err(PlanError::TooManyUserFlags {
            count: 31,
            maximum: 30
        })
    );
}
