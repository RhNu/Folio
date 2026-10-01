use super::*;

#[test]
fn generation_requires_exactly_one_publication_destination() {
    let arguments = [
        "folio",
        "declarations",
        "generate",
        "--source-root",
        "sources",
        "--source",
        "local-api",
    ];
    assert!(Cli::try_parse_from(arguments).is_err());
    let mut both = arguments.to_vec();
    both.extend(["--output", "api.fdecl", "--repo", "mod/api"]);
    assert!(Cli::try_parse_from(both).is_err());
    for destination in [["--output", "api.fdecl"], ["--repo", "mod/api"]] {
        let mut valid = arguments.to_vec();
        valid.extend(destination);
        let cli = Cli::try_parse_from(valid).unwrap();
        assert!(matches!(
            cli.command,
            Some(ProjectCommand::Declarations {
                command: DeclarationsCommand::Generate {
                    format: DeclarationOutputFormat::Binary,
                    ..
                }
            })
        ));
    }
}
