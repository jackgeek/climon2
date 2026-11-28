//! #501 (spec #488): the Pair layer of PAIR-01, PAIR-04 and PAIR-10. A
//! packaged relay and packaged hub monitors, each in its own climon2 home,
//! over loopback, with a real session daemon behind the relay. The on-path
//! proxy watches or edits what crosses, and the link peer stand-in plays a
//! relay that lies. Expectations come from the pairing ceremony (#193), the
//! stored pairings (#407) and the command surface (#478) resolutions.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[allow(dead_code)]
mod support;

use serde_json::Value;
use std::process::Output;
use support::{
    Package, TIMEOUT,
    pair::{RelayHome, fingerprint, host_name, links_file, messages, pair_in_terminal, pair_piped},
    proxy::{Direction, Proxy},
    stand_in::{Defect, StandInRelay},
};

/// The opening request's and reply's sizes, and where the purpose stands.
const REQUEST: usize = 26;
const REPLY: usize = 20;
const PURPOSE: usize = 8;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A failed `pair`: status 125, nothing on stdout, and `reason` on stderr.
fn assert_unpaired(output: &Output, reason: &str) {
    assert_eq!(output.status.code(), Some(125), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(text(&output.stderr), format!("climon2: error: {reason}\n"));
}

fn relays(hub: &Package) -> Option<Value> {
    links_file(hub, "relays.json")
}

/// The one relay `hub` remembers.
fn relay_record(hub: &Package) -> Value {
    let relays = relays(hub).expect("relays.json");
    let [relay] = relays["relays"]
        .as_array()
        .unwrap()
        .clone()
        .try_into()
        .unwrap();
    relay
}

fn hub_monitors(relay: &RelayHome) -> Vec<Value> {
    let file = links_file(&relay.package, "hub-monitors.json").expect("hub-monitors.json");
    file["hub_monitors"].as_array().unwrap().clone()
}

fn identity(relay: &RelayHome) -> Value {
    links_file(&relay.package, "relay-identity.json").expect("relay-identity.json")
}

/// PAIR-01 at the Pair layer, by command (P1, P4-5, P15-17, P19-23, CM15):
/// the relay mints, `c2 m p` takes the PIN typed with a hyphen at its
/// echoing prompt, and a second hub monitor pipes it with spaces. Pairing is
/// four messages on a connection of its own, then a link connection
/// follows. Each side shows the other's name and fingerprint, the hub
/// monitor only once its first link succeeded. Each hub monitor used a
/// fresh keypair; the relay kept its one identity and evicted no one.
#[test]
fn pair_01_c2_monitor_pair_pairs_and_then_links() {
    let mut relay = RelayHome::start();
    let before = identity(&relay);
    let relay_key = before["public_key"].as_str().unwrap().to_owned();
    let proxy = Proxy::faithful(relay.listen());
    let typed = proxy.address().to_string();

    let pin = relay.mint();
    let hub = Package::new();
    let mut terminal = pair_in_terminal(&hub, &typed);
    terminal.wait_for_text("PIN: ", TIMEOUT).unwrap();
    terminal.send(format!("{pin}\r")).unwrap();
    assert_eq!(
        terminal.wait_exit(TIMEOUT).unwrap(),
        0,
        "{}",
        terminal.screen()
    );
    let screen = terminal.screen();
    assert!(
        screen.contains(&format!("PIN: {pin}")),
        "the typing echoes: {screen}"
    );
    let paired = format!(
        "paired with {} (key fingerprint {})",
        host_name(),
        fingerprint(&relay_key)
    );
    assert!(screen.contains(&paired), "{screen}");

    let record = relay_record(&hub);
    assert_eq!(record["relay_public_key"], relay_key.as_str());
    assert_eq!(record["address"], typed.as_str(), "the address as typed");
    assert_eq!(record["disconnected"], false);
    assert!(
        record["last_linked_at"].is_u64(),
        "the first link succeeded"
    );
    let hub_key = record["public_key"].as_str().unwrap();
    relay.wait_for(&format!(" Key fingerprint {}", fingerprint(hub_key)));
    assert_eq!(relay.row(3), format!(" Paired with {}", host_name()));
    assert!(!relay.screen().contains(&pin), "the invitation is consumed");
    let remembered = hub_monitors(&relay);
    assert_eq!(remembered.len(), 1);
    assert_eq!(remembered[0]["public_key"], hub_key);

    let carried = proxy.carried();
    assert_eq!(carried.len(), 2, "a pairing connection, then a link");
    let pairing = &carried[0];
    assert_eq!(pairing.to_relay[PURPOSE], 1, "the first connection pairs");
    assert_eq!(messages(&pairing.to_relay[REQUEST..]).len(), 2);
    assert_eq!(messages(&pairing.to_hub[REPLY..]).len(), 2);
    assert_eq!(carried[1].to_relay[PURPOSE], 2, "the second one links");

    let second = Package::new();
    let pin = relay.mint();
    let spaced = format!(" {} ", pin.replace('-', " "));
    let output = pair_piped(
        &second,
        &relay.listen().to_string(),
        &format!("{spaced}\n"),
        &[],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(text(&output.stdout), format!("{paired}\n"));
    assert!(output.stderr.is_empty(), "no prompt without a terminal");
    let other = relay_record(&second);
    assert_ne!(other["public_key"], hub_key, "a fresh keypair");
    let remembered = hub_monitors(&relay);
    let keys: Vec<_> = remembered
        .iter()
        .map(|hub| hub["public_key"].clone())
        .collect();
    assert_eq!(keys, [Value::from(hub_key), other["public_key"].clone()]);
    assert_eq!(identity(&relay), before, "one identity, kept");

    drop(proxy);
    relay.stop();
    hub.assert_clean();
    second.assert_clean();
}

/// PAIR-04 by command (P25, CM24): an unreachable address, a relay with no
/// invitation open and a wrong PIN each fail with status 125 and say which;
/// nothing is remembered.
#[test]
fn pair_04_each_failure_is_named_with_125() {
    let mut relay = RelayHome::start();
    let hub = Package::new();
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().to_string()
    };
    let output = pair_piped(&hub, &closed, "12345678\n", &[]);
    assert_eq!(output.status.code(), Some(125), "{output:?}");
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with(&format!(
            "climon2: error: cannot reach the relay at {closed}: "
        )),
        "{stderr}"
    );

    let address = relay.listen().to_string();
    let output = pair_piped(&hub, &address, "12345678\n", &[]);
    assert_unpaired(
        &output,
        &format!("the relay at {address} has no link invitation open"),
    );

    let pin = relay.mint();
    let digits: String = pin.chars().filter(char::is_ascii_digit).collect();
    let last = digits.as_bytes()[7] - b'0';
    let wrong = format!("{}{}\n", &digits[..7], (last + 1) % 10);
    let output = pair_piped(&hub, &address, &wrong, &[]);
    assert_unpaired(
        &output,
        &format!("pairing with the relay at {address} failed: wrong PIN or interference"),
    );
    assert!(relays(&hub).is_none_or(|relays| relays["relays"] == Value::Array(Vec::new())));
    assert!(
        relay.screen().contains(&pin),
        "the invitation is still open"
    );
    assert!(links_file(&relay.package, "hub-monitors.json").is_none());
    relay.stop();
    hub.assert_clean();
}

/// PAIR-04 (P25), interference: the on-path proxy edits one byte of the hub
/// monitor's share, and the pairing fails as "wrong PIN or interference".
#[test]
fn pair_04_an_edited_share_fails_the_pairing() {
    let mut relay = RelayHome::start();
    const EDITED: usize = REQUEST + 2 + 7;
    let proxy = Proxy::tampering(relay.listen(), |chunk, bytes| {
        let mut bytes = bytes.to_vec();
        let range = chunk.offset..chunk.offset + bytes.len();
        if chunk.connection == 0 && chunk.direction == Direction::ToRelay && range.contains(&EDITED)
        {
            bytes[EDITED - chunk.offset] ^= 0x01;
        }
        bytes
    });
    let pin = relay.mint();
    let hub = Package::new();
    let address = proxy.address().to_string();
    let output = pair_piped(&hub, &address, &format!("{pin}\n"), &[]);
    assert_unpaired(
        &output,
        &format!("pairing with the relay at {address} failed: wrong PIN or interference"),
    );
    assert!(relays(&hub).is_none());
    drop(proxy);
    relay.stop();
    hub.assert_clean();
}

/// PAIR-10 (P13-14, CM10): with stdin a pipe there is no prompt and one
/// line is read. No flag, environment variable or setting carries a PIN.
/// The address is used and remembered as typed, a host name here, never
/// checked against the relay's hint.
#[test]
fn pair_10_the_pin_comes_only_from_the_prompt_or_stdin() {
    let mut relay = RelayHome::start();
    let hub = Package::new();
    let pin = relay.mint();
    let typed = format!("localhost:{}", relay.listen().port());

    let flagged = support::scripted(&hub, "c2", &["monitor", "pair", &typed, "--pin", &pin]);
    assert_eq!(flagged.status.code(), Some(2), "{flagged:?}");
    let from_env = pair_piped(&hub, &typed, "", &[("CLIMON2_PIN", &pin), ("PIN", &pin)]);
    assert_unpaired(
        &from_env,
        "a PIN is 8 digits; hyphens and spaces are ignored",
    );
    let config = hub.isolation.climon2_home().join("config.jsonc");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, format!(r#"{{"monitor": {{"pin": "{pin}"}}}}"#)).unwrap();
    let configured = pair_piped(&hub, &typed, "", &[]);
    assert_eq!(configured.status.code(), Some(125), "{configured:?}");
    assert!(configured.stdout.is_empty());
    std::fs::remove_file(&config).unwrap();
    assert!(
        relay.screen().contains(&pin),
        "the invitation is still open"
    );

    let output = pair_piped(&hub, &typed, &format!("{pin}\nignored\n"), &[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "no prompt on a pipe: {output:?}");
    assert_eq!(relay_record(&hub)["address"], typed.as_str());
    relay.wait_for(" Paired with ");
    relay.stop();
    hub.assert_clean();
}

/// PAIR-01 (P20): "paired" only once the first pinned-key link succeeds. A
/// stand-in relay pairs honestly, then answers the first link with another
/// key: the command fails, prints nothing on stdout and remembers nothing.
/// The exact stand-in pairs, which shows the defect is the only difference.
#[test]
fn pair_01_a_relay_whose_first_link_fails_is_not_reported_as_paired() {
    let hub = Package::new();
    let liar = StandInRelay::start(Defect::LinksWithAnotherKey);
    let address = liar.address().to_string();
    let output = pair_piped(&hub, &address, &format!("{}\n", liar.pin()), &[]);
    assert_unpaired(
        &output,
        &format!("pairing with the relay at {address} failed: its first link did not succeed"),
    );
    assert!(relays(&hub).is_none());
    liar.finish();

    let exact = StandInRelay::start(Defect::Exact);
    let output = pair_piped(&hub, &exact.address().to_string(), exact.pin(), &[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let key: String = exact
        .identity()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        text(&output.stdout),
        format!(
            "paired with stand-in (key fingerprint {})\n",
            fingerprint(&key)
        )
    );
    exact.finish();
    hub.assert_clean();
}
