use super::scaffold;

#[test]
fn names_produce_valid_nonreserved_starter_scripts() {
    let (script, manifest) = scaffold("CON").unwrap();
    assert_eq!(script, "FolioCON");
    assert!(manifest.contains("name = \"CON\""));
    let (script, _) = scaffold("my-mod").unwrap();
    assert_eq!(script, "FolioMyMod");
    assert!(scaffold("---").is_err());
    assert!(scaffold("../other").is_err());
}
