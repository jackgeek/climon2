//! #499 (spec #488, RLY-03, Contract layer): `c2 relay`'s arguments, the
//! `relay.listen` setting and which of them names the address a relay binds.
//! Every expectation is written here from the command-surface resolution
//! (#478, items 1 to 5), never taken from the code under test.
use climon2_core::{
    cli::{self, CliArgs},
    config::{ConfigError, ConfigSource, RelayConfig},
    layout::{ExecutableLayout, HomeLayout, VersionBinary},
    relay::{self, RelayArgs},
};
use std::{
    ffi::OsString,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
};

fn parse(args: &[&str]) -> Result<CliArgs, cli::UsageError> {
    CliArgs::parse(args.iter().map(OsString::from))
}

fn v4(octets: [u8; 4], port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::from(octets), port))
}

/// CM3: with nothing said, a relay listens on loopback, port 7222.
#[test]
fn the_default_listen_address_is_loopback_port_7222() {
    assert_eq!(relay::DEFAULT_LISTEN, v4([127, 0, 0, 1], 7222));
    assert_eq!(relay::DEFAULT_LISTEN.to_string(), "127.0.0.1:7222");
}

/// CM4: a listen value is an IP literal and a port. Port 0 is allowed.
#[test]
fn a_listen_value_is_an_ip_literal_and_a_port() {
    for (text, address) in [
        ("127.0.0.1:7222", v4([127, 0, 0, 1], 7222)),
        ("0.0.0.0:0", v4([0, 0, 0, 0], 0)),
        ("192.168.1.20:65535", v4([192, 168, 1, 20], 65535)),
        ("[::1]:7222", SocketAddr::from((Ipv6Addr::LOCALHOST, 7222))),
        ("[::]:0", SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))),
    ] {
        assert_eq!(relay::listen_address(text), Some(address), "{text}");
    }
}

/// CM4: host names and a missing port are refused, and so is anything else
/// that is not exactly an IP literal and a port.
#[test]
fn a_host_name_or_a_missing_port_is_no_listen_value() {
    for text in [
        "localhost:7222",
        "buildbox:7222",
        "example.com:7222",
        "127.0.0.1",
        "127.0.0.1:",
        "::1",
        "[::1]",
        "[::1]:",
        "::1:7222x",
        ":7222",
        "7222",
        "",
        " 127.0.0.1:7222",
        "127.0.0.1:7222 ",
        "127.0.0.1:65536",
        "127.0.0.1:-1",
        "256.0.0.1:7222",
        "tcp://127.0.0.1:7222",
    ] {
        assert_eq!(relay::listen_address(text), None, "{text:?}");
    }
}

/// CM1: `relay` and its shortcut `r` take nothing, or `--listen` and its
/// value.
#[test]
fn relay_and_r_take_no_arguments_or_one_listen_flag() {
    for name in ["relay", "r"] {
        assert_eq!(
            parse(&[name]).unwrap(),
            CliArgs::Relay(RelayArgs { listen: None })
        );
        assert_eq!(
            parse(&[name, "--listen", "0.0.0.0:0"]).unwrap(),
            CliArgs::Relay(RelayArgs {
                listen: Some(v4([0, 0, 0, 0], 0))
            })
        );
        assert_eq!(
            parse(&[name, "--listen", "[::1]:9000"]).unwrap(),
            CliArgs::Relay(RelayArgs {
                listen: Some(SocketAddr::from((Ipv6Addr::LOCALHOST, 9000)))
            })
        );
    }
}

/// CM5: an unknown flag or a malformed `--listen` is a usage error (2).
#[test]
fn an_unknown_flag_or_a_malformed_listen_is_a_usage_error() {
    for tail in [
        &["--port", "7222"][..],
        &["extra"],
        &[""],
        &["--help"],
        &["--listen"],
        &["--listen", "localhost:7222"],
        &["--listen", "127.0.0.1"],
        &["--listen", ""],
        &["--listen=127.0.0.1:7222"],
        &["--listen", "127.0.0.1:7222", "extra"],
        &["--listen", "127.0.0.1:7222", "--listen", "127.0.0.1:7223"],
        &["127.0.0.1:7222"],
        &["-l", "127.0.0.1:7222"],
    ] {
        for name in ["relay", "r"] {
            let args = [&[name][..], tail].concat();
            assert_eq!(parse(&args).unwrap_err().exit_code(), 2, "{args:?}");
        }
        let direct = RelayArgs::parse(tail.iter().map(OsString::from));
        assert_eq!(direct.unwrap_err().exit_code(), 2, "{tail:?}");
    }
}

/// A `--listen` value that is not Unicode is no address: a usage error.
#[cfg(unix)]
#[test]
fn a_listen_value_that_is_not_unicode_is_a_usage_error() {
    use std::os::unix::ffi::OsStringExt;
    let value = OsString::from_vec(b"127.0.0.1:72\xff".to_vec());
    let parsed = RelayArgs::parse([OsString::from("--listen"), value]);
    assert_eq!(parsed.unwrap_err().exit_code(), 2);
}

/// The CLI hands the relay executable the arguments it was given: what
/// `arguments` writes, `parse` reads back unchanged.
#[test]
fn relay_arguments_round_trip_through_the_relay_executable() {
    assert_eq!(RelayArgs { listen: None }.arguments(), [] as [OsString; 0]);
    let listening = RelayArgs {
        listen: Some(SocketAddr::from((Ipv6Addr::LOCALHOST, 0))),
    };
    assert_eq!(
        listening.arguments(),
        ["--listen", "[::1]:0"].map(OsString::from)
    );
    for args in [RelayArgs { listen: None }, listening] {
        assert_eq!(RelayArgs::parse(args.arguments()).unwrap(), args);
    }
}

/// CM2, CM3: the flag overrides the setting, and the default applies only
/// when neither is given.
#[test]
fn the_flag_overrides_the_setting_which_overrides_the_default() {
    let flag = v4([10, 0, 0, 1], 1);
    let setting = v4([10, 0, 0, 2], 2);
    let given = |listen| RelayArgs { listen };
    let configured = |listen| RelayConfig { listen };
    assert_eq!(
        given(None).listen(&configured(None)),
        v4([127, 0, 0, 1], 7222)
    );
    assert_eq!(given(None).listen(&configured(Some(setting))), setting);
    assert_eq!(given(Some(flag)).listen(&configured(None)), flag);
    assert_eq!(given(Some(flag)).listen(&configured(Some(setting))), flag);
}

/// CM2: `relay.listen` has the flag's form. A missing file, a missing
/// section and a missing setting all leave it unset.
#[test]
fn relay_listen_is_read_from_the_relay_section() {
    for text in ["{}", r#"{"relay": {}}"#, r#"{"monitor": {"bell": false}}"#] {
        assert_eq!(
            RelayConfig::parse(text.as_bytes()).unwrap(),
            RelayConfig { listen: None },
            "{text}"
        );
    }
    let text = br#"{
        // open to the LAN
        "relay": {"listen": "192.168.1.20:7300",},
    }"#;
    assert_eq!(
        RelayConfig::parse(text).unwrap(),
        RelayConfig {
            listen: Some(v4([192, 168, 1, 20], 7300))
        }
    );
    assert_eq!(
        RelayConfig::parse(br#"{"relay": {"listen": "[::]:0"}}"#).unwrap(),
        RelayConfig {
            listen: Some(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)))
        }
    );
}

/// CM4, CM5: a `relay.listen` that is no listen value is bad configuration,
/// and the refusal names the setting.
#[test]
fn a_malformed_relay_listen_is_invalid_configuration() {
    for value in [
        r#""localhost:7222""#,
        r#""127.0.0.1""#,
        r#""""#,
        "7222",
        "null",
        "true",
        r#"["127.0.0.1:7222"]"#,
    ] {
        let text = format!(r#"{{"relay": {{"listen": {value}}}}}"#);
        let Err(ConfigError::Invalid(detail)) = RelayConfig::parse(text.as_bytes()) else {
            panic!("{text} was not refused as invalid");
        };
        assert!(
            detail.contains(
                "relay.listen must be an IP address and a port, such as \"127.0.0.1:7222\""
            ),
            "{text}: {detail}"
        );
    }
    for text in [r#"{"relay": []}"#, r#"{"relay": "127.0.0.1:7222"}"#] {
        assert!(
            matches!(
                RelayConfig::parse(text.as_bytes()),
                Err(ConfigError::Invalid(_))
            ),
            "{text}"
        );
    }
    assert!(matches!(
        RelayConfig::parse(br#"{"relay": {"listen": "127.0.0.1:1"}"#),
        Err(ConfigError::Malformed(_))
    ));
}

/// The relay reads only its own section: a `monitor` or `client` section it
/// would refuse elsewhere does not stop it.
#[test]
fn the_relay_validates_only_its_own_section() {
    let text =
        br#"{"client": {"escape_prefix": 7}, "monitor": [], "relay": {"listen": "127.0.0.1:1"}}"#;
    assert_eq!(
        RelayConfig::parse(text).unwrap(),
        RelayConfig {
            listen: Some(v4([127, 0, 0, 1], 1))
        }
    );
}

/// CM2: the setting comes from the home's `config.jsonc`, and a missing file
/// sets nothing.
#[test]
fn the_relay_setting_is_read_from_the_home_configuration_file() {
    let home = std::env::temp_dir().join(format!("climon2-relay-config-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let layout = HomeLayout::at(&home).unwrap();
    let missing = ConfigSource::read(&layout).unwrap().relay().unwrap();
    assert_eq!(missing, RelayConfig { listen: None });
    std::fs::write(layout.config(), br#"{"relay": {"listen": "0.0.0.0:7222"}}"#).unwrap();
    let read = ConfigSource::read(&layout).unwrap().relay().unwrap();
    std::fs::remove_dir_all(&home).unwrap();
    assert_eq!(
        read,
        RelayConfig {
            listen: Some(v4([0, 0, 0, 0], 7222))
        }
    );
}

/// X1-2: the relay is `climon2-relay` in each version folder, found with the
/// layout code every sibling uses.
#[test]
fn the_relay_executable_lives_in_the_version_folder() {
    let root = std::env::current_dir().unwrap().join("install");
    let relay = root.join("2.0.0/climon2-relay");
    let layout = ExecutableLayout::from_binary_path(&relay, VersionBinary::Relay).unwrap();
    assert_eq!(layout.root(), root);
    assert_eq!(layout.version().as_str(), "2.0.0");
    let cli = ExecutableLayout::from_cli_path(&root.join("2.0.0/climon2-cli")).unwrap();
    assert_eq!(cli.binary(cli.version(), VersionBinary::Relay), relay);
    for bad in [
        "2.0.0/climon2-monitor",
        "2.0.0/climon2-relay-x",
        "climon2-relay",
    ] {
        assert!(
            ExecutableLayout::from_binary_path(&root.join(bad), VersionBinary::Relay).is_err(),
            "{bad}"
        );
    }
}

/// A home's monitor link files live in its `links` folder.
#[test]
fn the_links_folder_is_in_the_climon2_home() {
    let home = std::env::current_dir().unwrap().join("home");
    assert_eq!(HomeLayout::at(&home).unwrap().links(), home.join("links"));
}

/// CM1: the usage line shows the relay command with its one flag.
#[test]
fn the_usage_line_shows_relay_and_its_flag() {
    assert!(
        cli::USAGE.ends_with(" | relay|r [--listen <ip>:<port>]"),
        "{}",
        cli::USAGE
    );
}
