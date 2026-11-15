//! The terminal signals the detector interprets, parsed once from the
//! sequences the session's virtual terminal observed before filtering: OSC
//! 7501 program status, after the
//! [Program Status Protocol, revision 0.4](https://www.superlogical.com/rex/docs/build/program-status),
//! OSC 133 shell markers and full resets.
use climon2_vt::Signal as Observed;

/// A signal that drives detection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    /// A valid root OSC 7501 report.
    Root(Report),
    /// An OSC 133 shell marker.
    Shell(Marker),
    /// `ESC c` (RIS).
    FullReset,
}

/// The state a root OSC 7501 report gives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Report {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
    Clear,
}

/// An OSC 133 shell marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Marker {
    /// The prompt begins.
    A,
    /// The prompt ends; command-line input begins.
    B,
    /// The command's execution and output begin.
    C,
    /// The command finished.
    D,
}

/// What an observed sequence means to the session daemon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interpreted {
    /// It drives detection.
    Detect(Signal),
    /// An OSC 7501 capability query, which the daemon answers with
    /// [`QUERY_REPLY`] and which never drives detection.
    Query,
}

/// The daemon's local answer to an OSC 7501 capability query: every state
/// and kind of revision 0.4.
pub const QUERY_REPLY: &[u8] =
    b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

/// The longest OSC 7501 sequence, introducer and terminator included.
const MAX_SEQUENCE: usize = 4_096;
const MAX_KEY: usize = 16;
const MAX_APP: usize = 32;
/// Decoded limits. The encoded limits, 2,732 and 256 bytes, follow from
/// them: any longer base64 is invalid or decodes beyond them.
const MAX_MSG: usize = 2_048;
const MAX_TITLE: usize = 192;

const STATES: [(&[u8], Report); 6] = [
    (b"idle", Report::Idle),
    (b"working", Report::Working),
    (b"done", Report::Done),
    (b"blocked", Report::Blocked),
    (b"error", Report::Error),
    (b"clear", Report::Clear),
];

const MARKERS: [(&[u8], Marker); 4] = [
    (b"A", Marker::A),
    (b"B", Marker::B),
    (b"C", Marker::C),
    (b"D", Marker::D),
];

/// Base64 whose padding is optional, but correct when present.
const BASE64: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    base64::engine::GeneralPurposeConfig::new()
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
);

/// What `observed` means to the session daemon, if anything.
pub fn interpret(observed: Observed<'_>) -> Option<Interpreted> {
    match observed {
        Observed::FullReset => Some(Interpreted::Detect(Signal::FullReset)),
        Observed::Osc { body, length } => osc(body, length),
    }
}

fn osc(body: &[u8], length: usize) -> Option<Interpreted> {
    if let Some(report) = body.strip_prefix(b"7501;") {
        return program_status(report, length);
    }
    let marker = body.strip_prefix(b"133;").and_then(marker)?;
    Some(Interpreted::Detect(Signal::Shell(marker)))
}

/// The marker letter is the first parameter.
fn marker(parameters: &[u8]) -> Option<Marker> {
    let end = parameters.iter().position(|&byte| byte == b';');
    lookup(&MARKERS, &parameters[..end.unwrap_or(parameters.len())])
}

fn lookup<T: Copy>(table: &[(&[u8], T)], name: &[u8]) -> Option<T> {
    table
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, value)| *value)
}

fn program_status(report: &[u8], length: usize) -> Option<Interpreted> {
    if report == b"?" {
        return Some(Interpreted::Query);
    }
    let report = root(report).filter(|_| length <= MAX_SEQUENCE)?;
    Some(Interpreted::Detect(Signal::Root(report)))
}

/// The state of a valid report addressed to the root record, which has no
/// `id`. A report addressed to an `id` changes only records, which drive
/// nothing, so neither its `id` nor its other fields need validating.
fn root(report: &[u8]) -> Option<Report> {
    let fields = Fields::parse(report)?;
    if fields.get(b"id").is_some() || !fields.within_limits() {
        return None;
    }
    lookup(&STATES, fields.get(b"state")?)
}

/// A report's well-formed pairs, the last of a repeated key winning.
struct Fields<'a>(Vec<(&'a [u8], &'a [u8])>);

impl<'a> Fields<'a> {
    /// `None` when a key breaks its length limit, which discards the
    /// report; malformed pairs are skipped.
    fn parse(report: &'a [u8]) -> Option<Self> {
        let pairs: Vec<_> = report
            .split(|&byte| byte == b':')
            .filter_map(pair)
            .collect();
        let keys_within = pairs.iter().all(|(key, _)| key.len() <= MAX_KEY);
        keys_within.then_some(Self(pairs))
    }

    fn get(&self, key: &[u8]) -> Option<&'a [u8]> {
        self.0
            .iter()
            .rev()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| *value)
    }

    fn within_limits(&self) -> bool {
        self.get(b"app").is_none_or(|app| app.len() <= MAX_APP)
            && self.get(b"msg").is_none_or(|msg| text(msg, MAX_MSG))
            && self
                .get(b"title")
                .is_none_or(|title| text(title, MAX_TITLE))
    }
}

/// One well-formed `key=value` pair, whitespace around both removed: it
/// has an `=`, a key, and only value-set bytes in its value.
fn pair(raw: &[u8]) -> Option<(&[u8], &[u8])> {
    let at = raw.iter().position(|&byte| byte == b'=')?;
    let (key, value) = (raw[..at].trim_ascii(), raw[at + 1..].trim_ascii());
    (!key.is_empty() && value.iter().all(|&byte| valued(byte))).then_some((key, value))
}

/// A byte of the value set `[A-Za-z0-9_.,+/=-]`.
fn valued(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"_.,+/=-".contains(&byte)
}

/// Base64 of UTF-8 text of at most `max` bytes, without control characters.
fn text(encoded: &[u8], max: usize) -> bool {
    use base64::Engine;
    BASE64
        .decode(encoded)
        .ok()
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .is_some_and(|text| text.len() <= max && !text.chars().any(char::is_control))
}

#[cfg(test)]
mod tests {
    use super::{Interpreted, Marker, Report, Signal, interpret};
    use climon2_vt::Signal as Observed;

    /// What the OSC string `body` means, in a sequence of `length` bytes.
    fn sized(body: &str, length: usize) -> Option<Interpreted> {
        interpret(Observed::Osc {
            body: body.as_bytes(),
            length,
        })
    }

    fn osc(body: &str) -> Option<Interpreted> {
        sized(body, body.len() + 4)
    }

    fn root(report: Report) -> Option<Interpreted> {
        Some(Interpreted::Detect(Signal::Root(report)))
    }

    fn shell(marker: Marker) -> Option<Interpreted> {
        Some(Interpreted::Detect(Signal::Shell(marker)))
    }

    /// `state=done` with `pair` after it.
    fn done_with(pair: &str) -> String {
        format!("7501;state=done:{pair}")
    }

    fn encoded(text: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    /// STA-03 (pure, root reports): each of the six states of a root report,
    /// one without `id`, is that report; a full reset is one too.
    #[test]
    fn root_reports_and_full_resets_are_signals() {
        for (state, report) in [
            ("idle", Report::Idle),
            ("working", Report::Working),
            ("done", Report::Done),
            ("blocked", Report::Blocked),
            ("error", Report::Error),
            ("clear", Report::Clear),
        ] {
            assert_eq!(osc(&format!("7501;state={state}")), root(report), "{state}");
        }
        assert_eq!(
            interpret(Observed::FullReset),
            Some(Interpreted::Detect(Signal::FullReset))
        );
    }

    /// STA-03 (pure, queries and children): `?` alone is a capability query
    /// and nothing else is; a report addressed to an `id`, a child's, even
    /// an empty or invalid one, is no root report.
    #[test]
    fn queries_and_child_reports_are_no_root_reports() {
        assert_eq!(osc("7501;?"), Some(Interpreted::Query));
        for body in [
            "7501;state=working:id=build",
            "7501;id=build/test:state=done",
            "7501;state=blocked:id=",
            "7501;state=error:id=a/",
        ] {
            assert_eq!(osc(body), None, "{body}");
        }
    }

    /// STA-03 (pure, upstream validation): whitespace around keys and
    /// values is removed; the last of repeated keys wins; malformed pairs
    /// (no `=`, an empty key, or a value byte outside `[A-Za-z0-9_.,+/=-]`),
    /// unknown keys and invalid `kind` and `progress` are skipped while the
    /// rest applies, so a malformed `id` pair leaves a root report. A
    /// missing, empty or unknown state, or a misspelled report, is ignored.
    #[test]
    fn malformed_pairs_are_skipped_and_unknown_states_ignored() {
        for body in [
            "7501; state = done ",
            "7501;state=working:state=done",
            "7501;junk:=x:state=done:note=a b:other=x;y",
            "7501;state=done:id=a b",
            "7501;state=done:kind=unknown:progress=900:app=x.y-z_1:x=+/=,",
        ] {
            assert_eq!(osc(body), root(Report::Done), "{body}");
        }
        for body in [
            "7501;",
            "7501;state=",
            "7501;state=busy",
            "7501;state=Done",
            "7501;app=x",
            "7501;state=done!",
            "7501",
            "7502;state=done",
            "75011;state=done",
        ] {
            assert_eq!(osc(body), None, "{body}");
        }
    }

    /// STA-03 (pure, limits at the bound and one past): a report breaking a
    /// limit is discarded whole: a sequence over 4,096 bytes, a key over 16
    /// bytes, an `app` over 32 bytes, a `msg` decoding to over 2,048 bytes
    /// or a `title` to over 192.
    #[test]
    fn reports_breaking_a_limit_are_discarded() {
        assert_eq!(sized("7501;state=done", 4_096), root(Report::Done));
        assert_eq!(sized("7501;state=done", 4_097), None);
        let msg = |bytes: usize| format!("msg={}", encoded(&vec![b'm'; bytes]));
        let title = |bytes: usize| format!("title={}", encoded(&vec![b't'; bytes]));
        let within = [
            format!("{}=1", "k".repeat(16)),
            format!("app={}", "a".repeat(32)),
            msg(2_048),
            title(192),
        ];
        let beyond = [
            format!("{}=1", "k".repeat(17)),
            format!("app={}", "a".repeat(33)),
            msg(2_049),
            title(193),
        ];
        for (within, beyond) in within.iter().zip(&beyond) {
            assert_eq!(osc(&done_with(within)), root(Report::Done), "{within}");
            assert_eq!(osc(&done_with(beyond)), None, "{beyond}");
        }
    }

    /// STA-03 (pure, text values): `msg` and `title` are base64 of UTF-8
    /// text without control characters, padding optional. Undecodable
    /// base64, text that is not UTF-8, and any control character (C0,
    /// DEL or C1, newline and tab included) discard the report.
    #[test]
    fn text_values_must_be_base64_utf8_without_controls() {
        for pair in ["msg=SGk", "msg=SGk=", "title=w6k=", "msg="] {
            assert_eq!(osc(&done_with(pair)), root(Report::Done), "{pair}");
        }
        let controls: [&[u8]; 5] = [b"a\nb", b"a\tb", b"\x7f", "\u{85}".as_bytes(), b"\xff"];
        for text in controls {
            for key in ["msg", "title"] {
                let pair = format!("{key}={}", encoded(text));
                assert_eq!(osc(&done_with(&pair)), None, "{pair}");
            }
        }
        for pair in ["msg=a", "title=SGk==", "msg=S=Gk"] {
            assert_eq!(osc(&done_with(pair)), None, "{pair}");
        }
    }

    /// STA-05 (pure, shell markers): OSC 133 `A`, `B`, `C` and `D` are the
    /// four markers, with or without further parameters such as `D`'s exit
    /// status; any other OSC 133 or other OSC is no signal.
    #[test]
    fn osc_133_markers_are_shell_signals() {
        for (body, marker) in [
            ("133;A", Marker::A),
            ("133;B", Marker::B),
            ("133;C", Marker::C),
            ("133;D", Marker::D),
            ("133;D;1", Marker::D),
            ("133;A;cl=m", Marker::A),
        ] {
            assert_eq!(osc(body), shell(marker), "{body}");
        }
        for body in [
            "133;",
            "133;E",
            "133;AB",
            "133;P;k=i",
            "133",
            "1330;A",
            "2;title",
            "",
        ] {
            assert_eq!(osc(body), None, "{body}");
        }
    }
}
