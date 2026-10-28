//! #29 recovered baseline / #17: exact argv and inherited session environment.
use climon2_core::{cli::CliArgs, session::*};
use std::ffi::{OsStr, OsString};

fn environment(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).into(), (*value).into()))
        .collect()
}

fn value<'a>(env: &'a SessionEnvironment, name: &str) -> &'a OsStr {
    &env.values.iter().find(|(key, _)| key == name).unwrap().1
}

#[test]
fn issue29_exec_preserves_literal_os_arguments_and_list_rejects_usage_errors() {
    let args = ["exec", "some command", "--flag", "", "a b", "日本語", "--"];
    let CliArgs::Exec(command) = CliArgs::parse(args.map(OsString::from)).unwrap() else {
        panic!("exec")
    };
    assert_eq!(command.command, "some command");
    assert_eq!(
        command.args,
        args[2..].iter().map(OsString::from).collect::<Vec<_>>()
    );
    let CliArgs::Exec(literal) =
        CliArgs::parse(["exec", "--", "-command", ""].map(OsString::from)).unwrap()
    else {
        panic!("exec")
    };
    assert_eq!(literal.command, "-command");
    assert_eq!(literal.args, [OsString::new()]);
    assert_eq!(
        CliArgs::parse(["list", "--json"].map(OsString::from)).unwrap(),
        CliArgs::List { json: true }
    );
    assert_eq!(
        CliArgs::parse(["list"].map(OsString::from)).unwrap(),
        CliArgs::List { json: false }
    );
    for bad in [
        vec![],
        vec!["exec"],
        vec!["exec", "--"],
        vec!["unknown"],
        vec!["list", "extra"],
        vec!["list", "--json", "--json"],
    ] {
        assert_eq!(
            CliArgs::parse(bad.into_iter().map(OsString::from))
                .unwrap_err()
                .exit_code(),
            2
        );
    }
}

#[test]
fn issue29_top_level_ignores_stray_depth_and_sets_only_session_vars_and_absent_term() {
    let id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    let inherited = environment(&[
        ("CLIMON2_SESSION_DEPTH", "88"),
        ("TOKEN", "literal\n日本語"),
        ("CLIMON2_HOME", "unchanged"),
    ]);
    let env = SessionEnvironment::for_session(&id, &inherited).unwrap();
    assert_eq!(env.depth.get(), 1);
    assert_eq!(env.nested_warning, None);
    assert_eq!(value(&env, "CLIMON2_SESSION_ID"), id.as_str());
    assert_eq!(value(&env, "CLIMON2_SESSION_DEPTH"), "1");
    assert_eq!(value(&env, "TERM"), "xterm-256color");
    assert_eq!(value(&env, "TOKEN"), "literal\n日本語");
    assert_eq!(value(&env, "CLIMON2_HOME"), "unchanged");
    assert_eq!(inherited[0].1, "88");
}

#[test]
fn issue29_nested_depth_defaults_malformed_enclosing_to_one_and_preserves_empty_term() {
    let id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    for malformed in ["", "no", "0", "-1", " 2", "+2", "4294967296"] {
        let inherited = environment(&[
            ("CLIMON2_SESSION_ID", "outer"),
            ("CLIMON2_SESSION_DEPTH", malformed),
            ("TERM", ""),
        ]);
        let env = SessionEnvironment::for_session(&id, &inherited).unwrap();
        assert_eq!(env.depth.get(), 2, "{malformed}");
        assert_eq!(value(&env, "TERM"), "");
        assert_eq!(
            env.nested_warning.as_deref(),
            Some("Starting climon2 session nested for the 1st time")
        );
    }
    let env =
        SessionEnvironment::for_session(&id, &environment(&[("CLIMON2_SESSION_ID", "outer")]))
            .unwrap();
    assert_eq!(env.depth.get(), 2);
    let env = SessionEnvironment::for_session(
        &id,
        &environment(&[("CLIMON2_SESSION_ID", ""), ("CLIMON2_SESSION_DEPTH", "7")]),
    )
    .unwrap();
    assert_eq!(env.depth.get(), 1);
}

#[test]
fn issue29_nested_ordinals_include_teens_centuries_and_explicit_depth_overflow() {
    let id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    for (enclosing, ordinal) in [
        (1, "1st"),
        (2, "2nd"),
        (3, "3rd"),
        (4, "4th"),
        (11, "11th"),
        (12, "12th"),
        (13, "13th"),
        (21, "21st"),
        (22, "22nd"),
        (23, "23rd"),
        (111, "111th"),
        (112, "112th"),
        (113, "113th"),
        (4_294_967_294, "4294967294th"),
    ] {
        let inherited = environment(&[
            ("CLIMON2_SESSION_ID", "outer"),
            ("CLIMON2_SESSION_DEPTH", &enclosing.to_string()),
        ]);
        let env = SessionEnvironment::for_session(&id, &inherited).unwrap();
        assert_eq!(env.depth.get(), enclosing + 1);
        assert_eq!(
            env.nested_warning.unwrap(),
            format!("Starting climon2 session nested for the {ordinal} time")
        );
    }
    let inherited = environment(&[
        ("CLIMON2_SESSION_ID", "outer"),
        ("CLIMON2_SESSION_DEPTH", "4294967295"),
    ]);
    assert_eq!(
        SessionEnvironment::for_session(&id, &inherited).unwrap_err(),
        DepthOverflow
    );
    assert_eq!(ordinal(0), "0th");
    assert_eq!(ordinal(u32::MAX), "4294967295th");
}
