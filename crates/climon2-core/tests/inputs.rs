//! #29 recovered baseline / #17: exact argv and inherited session environment.
use climon2_core::{cli::CliArgs, selector::Selector, session::*};
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

/// #29: exec keeps its OS arguments literally, list, attach and kill parse
/// exactly, and anything else they are given is a usage error (2).
#[test]
fn issue29_exec_list_attach_and_kill_parse_exact_arguments_and_reject_usage_errors() {
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
    assert_eq!(
        CliArgs::parse(["attach", "01234567"].map(OsString::from)).unwrap(),
        CliArgs::Attach(Selector::parse(OsStr::new("01234567")).unwrap())
    );
    assert_eq!(
        CliArgs::parse(["kill", "01234567"].map(OsString::from)).unwrap(),
        CliArgs::Kill(Selector::parse(OsStr::new("01234567")).unwrap())
    );
    for bad in [
        vec![],
        vec!["exec"],
        vec!["exec", "--"],
        vec!["exec", ""],
        vec!["exec", "--", ""],
        vec!["unknown"],
        vec!["list", "extra"],
        vec!["list", "--json", "--json"],
        vec!["attach"],
        vec!["attach", "short"],
        vec!["attach", "01234567", "extra"],
        vec!["kill"],
        vec!["kill", "short"],
        vec!["kill", "01234567", "extra"],
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
fn attach_and_kill_accept_short_selectors_but_not_empty_or_extra_arguments() {
    for operation in ["attach", "kill"] {
        for (text, expected) in [
            ("A", "a"),
            ("-AB-", "ab"),
            ("ABC", "abc"),
            ("ABCD", "abcd"),
            ("ABCDE", "abcde"),
            ("ABCDEF", "abcdef"),
            ("ABCDE-F0", "abcdef0"),
        ] {
            let parsed = CliArgs::parse([operation, text].map(OsString::from)).unwrap();
            let selector = match (operation, parsed) {
                ("attach", CliArgs::Attach(selector)) | ("kill", CliArgs::Kill(selector)) => {
                    selector
                }
                (_, other) => panic!("unexpected command: {other:?}"),
            };
            assert_eq!(selector.as_str(), expected);
        }
        for arguments in [
            vec![operation, ""],
            vec![operation, "---"],
            vec![operation, "g"],
            vec![operation, "a", "extra"],
        ] {
            assert_eq!(
                CliArgs::parse(arguments.into_iter().map(OsString::from))
                    .unwrap_err()
                    .exit_code(),
                2
            );
        }
    }
}

/// #74 (INP-01 derived refusal): `monitor` and its alias `mon` take no
/// arguments; anything after either word is a usage error (2).
#[test]
fn issue74_monitor_and_mon_take_no_arguments() {
    for monitor in ["monitor", "mon"] {
        assert_eq!(
            CliArgs::parse([monitor].map(OsString::from)).unwrap(),
            CliArgs::Monitor
        );
    }
    for bad in [["monitor", "extra"], ["mon", ""], ["mon", "--help"]] {
        assert_eq!(
            CliArgs::parse(bad.map(OsString::from))
                .unwrap_err()
                .exit_code(),
            2
        );
    }
}

/// #29: help and version flags, long or short, parse only when given alone.
#[test]
fn issue29_help_and_version_flags_take_no_arguments() {
    for (flag, expected) in [
        ("--help", CliArgs::Help),
        ("-h", CliArgs::Help),
        ("--version", CliArgs::Version),
        ("-V", CliArgs::Version),
    ] {
        assert_eq!(CliArgs::parse([OsString::from(flag)]).unwrap(), expected);
        assert!(CliArgs::parse([flag, "extra"].map(OsString::from)).is_err());
    }
}

#[test]
fn issue29_top_level_ignores_stray_depth_and_sets_session_vars_and_terminal_identity() {
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
    assert_eq!(value(&env, "COLORTERM"), "truecolor");
    assert_eq!(value(&env, "TOKEN"), "literal\n日本語");
    assert_eq!(value(&env, "CLIMON2_HOME"), "unchanged");
    assert_eq!(inherited[0].1, "88");
}

/// Plan 1.17: the command's terminal identity replaces any inherited value and
/// the client startup envelope never reaches the daemon or the command.
#[test]
fn projected_environment_replaces_term_and_colorterm_and_drops_startup_envelope() {
    let id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    for (term, colorterm) in [("", ""), ("dumb", "24bit"), ("xterm-256color", "truecolor")] {
        let inherited = environment(&[
            ("TERM", term),
            ("A", "1"),
            ("COLORTERM", colorterm),
            (
                "CLIMON2_CLIENT_STARTUP",
                r#"{"schema":1,"operation":"kill"}"#,
            ),
            ("TERM", "second"),
            ("COLORTERM", "second"),
            ("CLIMON2_CLIENT_STARTUP", "second"),
            ("TERM_PROGRAM", "kept"),
            ("Z", "2"),
        ]);
        let env = SessionEnvironment::for_session(&id, &inherited).unwrap();
        let keys: Vec<_> = env
            .values
            .iter()
            .map(|(key, _)| key.to_str().unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                "A",
                "TERM_PROGRAM",
                "Z",
                "CLIMON2_SESSION_ID",
                "CLIMON2_SESSION_DEPTH",
                "TERM",
                "COLORTERM"
            ],
            "{term:?}"
        );
        assert_eq!(value(&env, "TERM"), "xterm-256color");
        assert_eq!(value(&env, "COLORTERM"), "truecolor");
        assert_eq!(value(&env, "TERM_PROGRAM"), "kept");
    }
}

#[test]
fn issue29_nested_depth_defaults_malformed_enclosing_to_one_and_replaces_empty_term() {
    let id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    for malformed in ["", "no", "0", "-1", " 2", "+2", "4294967296"] {
        let inherited = environment(&[
            ("CLIMON2_SESSION_ID", "outer"),
            ("CLIMON2_SESSION_DEPTH", malformed),
            ("TERM", ""),
        ]);
        let env = SessionEnvironment::for_session(&id, &inherited).unwrap();
        assert_eq!(env.depth.get(), 2, "{malformed}");
        assert_eq!(value(&env, "TERM"), "xterm-256color");
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
