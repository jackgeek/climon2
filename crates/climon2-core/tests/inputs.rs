//! #29 recovered baseline / #17: exact argv and inherited session environment.
use climon2_core::{
    cli::{self, CliArgs, CommandArgs},
    lifetime::Lifetime,
    selector::Selector,
    session::*,
};
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
    let CliArgs::Exec {
        lifetime: Lifetime::Ordinary,
        command,
    } = CliArgs::parse(args.map(OsString::from)).unwrap()
    else {
        panic!("exec")
    };
    assert_eq!(command.command, "some command");
    assert_eq!(
        command.args,
        args[2..].iter().map(OsString::from).collect::<Vec<_>>()
    );
    let CliArgs::Exec {
        lifetime: Lifetime::Ordinary,
        command: literal,
    } = CliArgs::parse(["exec", "--", "-command", ""].map(OsString::from)).unwrap()
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

/// Confirmed scenario (headless sessions, #73/#78): `exec --headless [--]
/// <command>` creates a headless session. `--` still ends climon2's options,
/// so `exec -- --headless` runs a command literally named `--headless`, and
/// nothing after the command is interpreted.
#[test]
fn exec_headless_selects_the_headless_lifetime_and_double_dash_keeps_it_literal() {
    let parse = |args: &[&str]| CliArgs::parse(args.iter().map(OsString::from));
    let exec = |lifetime, command: &str, args: &[&str]| CliArgs::Exec {
        lifetime,
        command: CommandArgs {
            command: command.into(),
            args: args.iter().map(OsString::from).collect(),
        },
    };
    let headless = Lifetime::Headless;
    let ordinary = Lifetime::Ordinary;
    for (args, expected) in [
        (
            &["exec", "--headless", "cmd", "--headless", ""][..],
            exec(headless, "cmd", &["--headless", ""]),
        ),
        (
            &["exec", "--headless", "--", "--headless"],
            exec(headless, "--headless", &[]),
        ),
        (
            &["exec", "--headless", "--", "cmd", "--"],
            exec(headless, "cmd", &["--"]),
        ),
        (
            &["exec", "--", "--headless", "cmd"],
            exec(ordinary, "--headless", &["cmd"]),
        ),
        (
            &["exec", "cmd", "--headless"],
            exec(ordinary, "cmd", &["--headless"]),
        ),
    ] {
        assert_eq!(parse(args).unwrap(), expected, "{args:?}");
    }
    for bad in [
        &["exec", "--headless"][..],
        &["exec", "--headless", "--"],
        &["exec", "--headless", ""],
        &["exec", "--headless", "--", ""],
    ] {
        assert_eq!(parse(bad).unwrap_err().exit_code(), 2, "{bad:?}");
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

/// #74 (INP-01 derived refusal): `monitor` and its shortcut `m` take no
/// arguments; anything after either is a usage error (2). #287: the old alias
/// `mon` is gone and is a usage error too.
#[test]
fn issue74_monitor_and_m_take_no_arguments_and_mon_is_gone() {
    for monitor in ["monitor", "m"] {
        assert_eq!(
            CliArgs::parse([monitor].map(OsString::from)).unwrap(),
            CliArgs::Monitor
        );
    }
    for bad in [
        &["monitor", "extra"][..],
        &["m", ""],
        &["m", "--help"],
        &["mon"],
    ] {
        assert_eq!(
            CliArgs::parse(bad.iter().map(OsString::from))
                .unwrap_err()
                .exit_code(),
            2,
            "{bad:?}"
        );
    }
}

/// #133 / ADR 0011: `home` and its shortcut `h` take nothing, or `release`
/// and then optionally `--force`; anything else is a usage error (2).
#[test]
fn issue133_home_takes_nothing_or_release() {
    for home in ["home", "h"] {
        let parse = |tail: &[&str]| {
            CliArgs::parse([&[home][..], tail].concat().into_iter().map(OsString::from))
        };
        assert_eq!(parse(&[]).unwrap(), CliArgs::Home);
        assert_eq!(
            parse(&["release"]).unwrap(),
            CliArgs::HomeRelease { force: false }
        );
        assert_eq!(
            parse(&["release", "--force"]).unwrap(),
            CliArgs::HomeRelease { force: true }
        );
        for bad in [
            &["release", "extra"][..],
            &["release", "--force", "extra"],
            &["--force"],
            &["--force", "release"],
            &["release", "-f"],
            &["Release"],
            &[""],
            &["--help"],
            &["show"],
        ] {
            assert_eq!(parse(bad).unwrap_err().exit_code(), 2, "{home} {bad:?}");
        }
    }
}

/// #133 / ADR 0011: `--force`, as the first argument only, makes exec, attach,
/// kill, list and monitor use a home another host owns; the rest parses as
/// without it. A second `--force` is exec's command, as a second option is,
/// and a usage error elsewhere; `home` takes it only after `release`.
#[test]
fn issue133_force_comes_first_on_every_command_the_owner_check_can_refuse() {
    let parse = |args: &[&str]| CliArgs::parse(args.iter().map(OsString::from));
    for tail in [
        &["exec", "make", "test"][..],
        &["exec", "--headless", "make"],
        &["x", "-h", "--", "--force"],
        &["attach", "01234567"],
        &["a", "ABCDE-F0"],
        &["kill", "01234567"],
        &["list"],
        &["list", "--json"],
        &["l", "--json"],
        &["monitor"],
        &["m"],
    ] {
        let forced = [&[tail[0], "--force"][..], &tail[1..]].concat();
        let (command, force) = parse(&forced).unwrap().unforced();
        assert_eq!(command, parse(tail).unwrap(), "{forced:?}");
        assert!(force, "{forced:?}");
        assert!(!parse(tail).unwrap().unforced().1, "{tail:?}");
    }
    let (command, force) = parse(&["exec", "--force", "--force", "cmd"])
        .unwrap()
        .unforced();
    assert!(force);
    assert_eq!(
        command,
        CliArgs::Exec {
            lifetime: Lifetime::Ordinary,
            command: CommandArgs {
                command: "--force".into(),
                args: vec!["cmd".into()],
            },
        }
    );
    let exec = parse(&["exec", "-h", "--force", "cmd"]).unwrap().unforced();
    assert!(!exec.1, "--force after -h is the command");
    for bad in [
        &["list", "--force", "--force"][..],
        &["list", "--json", "--force"],
        &["monitor", "--force", "extra"],
        &["attach", "--force"],
        &["kill", "01234567", "--force"],
        &["home", "--force"],
        &["h", "--force", "release"],
        &["--force", "list"],
        &["--force"],
    ] {
        assert_eq!(parse(bad).unwrap_err().exit_code(), 2, "{bad:?}");
    }
}

/// #29: help and version flags parse only when given alone. #287: help has
/// only its long flag, so `-h` alone is a usage error (2).
#[test]
fn issue29_help_and_version_flags_take_no_arguments() {
    assert_eq!(
        CliArgs::parse([OsString::from("-h")])
            .unwrap_err()
            .exit_code(),
        2
    );
    for (flag, expected) in [
        ("--help", CliArgs::Help),
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

/// #287: each command and its single-letter shortcut, as the spec lists them.
const SHORTCUTS: [(&str, &str); 6] = [
    ("exec", "x"),
    ("attach", "a"),
    ("kill", "k"),
    ("list", "l"),
    ("monitor", "m"),
    ("home", "h"),
];

/// #287: every command's single-letter shortcut is a pure synonym: given any
/// arguments, valid or not, it parses exactly as the full name does.
#[test]
fn shortcuts_parse_exactly_as_their_full_commands() {
    let parse = |args: &[&str]| CliArgs::parse(args.iter().map(OsString::from));
    let tails: [&[&str]; 25] = [
        &[],
        &["cmd", "--flag", ""],
        &["--", "-cmd"],
        &["--headless"],
        &["--headless", "make", "test"],
        &["--headless", "--", "--headless"],
        &["-h"],
        &["-h", "make", "-h"],
        &["-h", "--", ""],
        &["--json"],
        &["--json", "--json"],
        &["01234567"],
        &["ABCDE-F0"],
        &["01234567", "extra"],
        &["short"],
        &[""],
        &["--help"],
        &["extra"],
        &["release"],
        &["release", "extra"],
        &["release", "--force"],
        &["--force"],
        &["--force", "--json"],
        &["--force", "01234567"],
        &["--force", "--headless", "make"],
    ];
    for (full, shortcut) in SHORTCUTS {
        let mut accepted = 0;
        for tail in tails {
            let long = [&[full][..], tail].concat();
            let short = [&[shortcut][..], tail].concat();
            assert_eq!(parse(&short), parse(&long), "{short:?}");
            accepted += usize::from(parse(&short).is_ok());
        }
        assert!(accepted > 0, "{shortcut} accepted nothing");
    }
}

/// #287: `-h` is `--headless`'s short form under exactly its rules: only as
/// exec's first argument, and `--` still ends climon2's options.
#[test]
fn exec_dash_h_is_headless_only_as_the_first_argument() {
    let parse = |args: &[&str]| CliArgs::parse(args.iter().map(OsString::from));
    let exec = |lifetime, command: &str, args: &[&str]| CliArgs::Exec {
        lifetime,
        command: CommandArgs {
            command: command.into(),
            args: args.iter().map(OsString::from).collect(),
        },
    };
    let headless = Lifetime::Headless;
    let ordinary = Lifetime::Ordinary;
    for exec_name in ["exec", "x"] {
        for (tail, expected) in [
            (
                &["-h", "make", "test"][..],
                exec(headless, "make", &["test"]),
            ),
            (&["-h", "cmd", "-h", ""], exec(headless, "cmd", &["-h", ""])),
            (&["-h", "--", "-h"], exec(headless, "-h", &[])),
            (&["-h", "--", "cmd", "--"], exec(headless, "cmd", &["--"])),
            (&["--", "-h", "cmd"], exec(ordinary, "-h", &["cmd"])),
            (&["make", "-h"], exec(ordinary, "make", &["-h"])),
            // Only one option: a second is the command.
            (
                &["-h", "--headless", "cmd"],
                exec(headless, "--headless", &["cmd"]),
            ),
        ] {
            let args = [&[exec_name][..], tail].concat();
            assert_eq!(parse(&args).unwrap(), expected, "{args:?}");
        }
        for tail in [&["-h"][..], &["-h", "--"], &["-h", ""], &["-h", "--", ""]] {
            let args = [&[exec_name][..], tail].concat();
            assert_eq!(parse(&args).unwrap_err().exit_code(), 2, "{args:?}");
        }
    }
}

/// #287: every command has exactly one single-letter shortcut, no two
/// commands share one, no shortcut is another command's name, and `r` stays
/// reserved for `relay`.
#[test]
fn every_command_has_its_own_single_letter_shortcut() {
    let commands: Vec<_> = cli::commands().collect();
    assert_eq!(commands, SHORTCUTS);
    let mut names: Vec<_> = commands
        .iter()
        .flat_map(|(full, shortcut)| [*full, *shortcut])
        .collect();
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        count,
        "a shortcut or name is shared: {commands:?}"
    );
    for (full, shortcut) in &commands {
        assert_eq!(shortcut.chars().count(), 1, "{full}");
        assert_ne!(*shortcut, "r", "{full} takes relay's reserved shortcut");
    }
    // The usage line names exactly these commands, each beside its shortcut.
    let named: Vec<_> = cli::USAGE
        .trim_start_matches("usage: c2 ")
        .split(" | ")
        .map(|form| form.split(' ').next().unwrap())
        .collect();
    let expected: Vec<_> = SHORTCUTS
        .iter()
        .map(|(full, shortcut)| format!("{full}|{shortcut}"))
        .collect();
    assert_eq!(named, expected, "{}", cli::USAGE);
    assert!(cli::USAGE.contains("[--headless|-h]"), "{}", cli::USAGE);
    assert_eq!(
        CliArgs::parse(["r"].map(OsString::from))
            .unwrap_err()
            .exit_code(),
        2,
        "r is reserved, not a command"
    );
}
