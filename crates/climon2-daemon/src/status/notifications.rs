//! Notification sources, parsed once from the OSC strings the session's
//! virtual terminal observed, after the agreed
//! [source normalization](../../../../docs/for-humans/protocol/daemon-client/README.md#source-normalization):
//! [OSC 9](https://iterm2.com/documentation-escape-codes.html) text, the
//! [OSC 777](https://codeberg.org/dnkl/foot/src/branch/master/doc/foot-ctlseqs.7.scd)
//! `notify` subset and
//! [OSC 99](https://sw.kovidgoyal.net/kitty/desktop-notifications/) chunks.
//! Text stays raw bytes here, base64 still encoded; the assembly decodes
//! it and makes it inert UTF-8.
use climon2_core::wire::NotificationSource;

/// What a notification sequence carries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Parsed<'a> {
    /// A complete notification in one sequence: OSC 9 text, a body, or
    /// OSC 777 `notify`, a title and an optional body.
    Complete {
        source: NotificationSource,
        title: Option<&'a [u8]>,
        body: Option<&'a [u8]>,
    },
    /// One OSC 99 chunk of a logical notification.
    Chunk(Chunk<'a>),
}

/// One OSC 99 chunk: its notification's identifier, whether it completes
/// that notification, what its payload is, whether the payload is base64,
/// and the payload as it arrived.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chunk<'a> {
    pub id: Option<&'a [u8]>,
    pub done: bool,
    pub part: Part,
    pub base64: bool,
    pub payload: &'a [u8],
}

/// What an OSC 99 chunk's payload is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Part {
    Title,
    Body,
    /// Data that only accompanies a notification, such as an icon or
    /// buttons: it never makes one on its own.
    Auxiliary,
}

/// The parameters of OSC 9, after `9;`, when they are a text
/// notification: its whole text. A first parameter of decimal digits is a
/// [ConEmu](https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC)
/// control operation, progress (`4`) included, never a notification.
pub fn osc9(parameters: &[u8]) -> Option<Parsed<'_>> {
    let (first, _) = split_once(parameters, b';');
    let control = !first.is_empty() && first.iter().all(u8::is_ascii_digit);
    (!control).then_some(Parsed::Complete {
        source: NotificationSource::Osc9,
        title: None,
        body: Some(parameters),
    })
}

/// The parameters of OSC 777, after `777;`, when they are `notify`: the
/// title runs to the first `;`, and the body, if any, is the rest, later
/// semicolons included. Other subcommands are no notifications.
pub fn osc777(parameters: &[u8]) -> Option<Parsed<'_>> {
    let (title, body) = split_once(parameters.strip_prefix(b"notify;")?, b';');
    Some(Parsed::Complete {
        source: NotificationSource::Osc777,
        title: Some(title),
        body,
    })
}

/// `text` up to the first `delimiter`, and the rest after it, if any.
fn split_once(text: &[u8], delimiter: u8) -> (&[u8], Option<&[u8]>) {
    match text.iter().position(|&byte| byte == delimiter) {
        Some(at) => (&text[..at], Some(&text[at + 1..])),
        None => (text, None),
    }
}

/// The parameters of OSC 99, after `99;`: `metadata;payload`, the metadata
/// colon-separated `key=value` pairs. `i` is the identifier, `d=0` leaves
/// the notification incomplete (`d=1`, the default, completes it), `p`
/// names the payload (`title`, the default, `body`, or auxiliary `icon`
/// and `buttons`) and `e=1` makes it base64, which the assembly decodes.
/// Queries (`p=?`), `close` and `alive` are no notification data. A
/// missing payload, an invalid value or identifier, and an unknown payload
/// type discard the chunk; unknown keys are ignored.
pub fn osc99(parameters: &[u8]) -> Option<Parsed<'_>> {
    let (metadata, payload) = split_once(parameters, b';');
    let metadata = metadata
        .split(|&byte| byte == b':')
        .try_fold(Metadata::default(), Metadata::apply)?;
    metadata.chunk(payload?)
}

/// An OSC 99 chunk's metadata, its defaults until a pair overrides them.
/// The part is `None` for data that is no notification's.
struct Metadata<'a> {
    id: Option<&'a [u8]>,
    done: bool,
    part: Option<Part>,
    base64: bool,
}

impl Default for Metadata<'_> {
    fn default() -> Self {
        Self {
            id: None,
            done: true,
            part: Some(Part::Title),
            base64: false,
        }
    }
}

const PARTS: [(&[u8], Option<Part>); 7] = [
    (b"title", Some(Part::Title)),
    (b"body", Some(Part::Body)),
    (b"icon", Some(Part::Auxiliary)),
    (b"buttons", Some(Part::Auxiliary)),
    (b"?", None),
    (b"close", None),
    (b"alive", None),
];

/// Sets one key's value; `None` discards the chunk.
type Setter = for<'a> fn(&mut Metadata<'a>, &'a [u8]) -> Option<()>;

const KEYS: [(&[u8], Setter); 4] = [
    (b"i", set_id),
    (b"d", set_done),
    (b"p", set_part),
    (b"e", set_base64),
];

fn set_id<'a>(metadata: &mut Metadata<'a>, id: &'a [u8]) -> Option<()> {
    let valid = !id.is_empty()
        && id
            .iter()
            .all(|&byte| byte.is_ascii_alphanumeric() || b"-_+.".contains(&byte));
    metadata.id = Some(id);
    valid.then_some(())
}

fn set_done(metadata: &mut Metadata<'_>, value: &[u8]) -> Option<()> {
    metadata.done = flag(value)?;
    Some(())
}

fn set_part(metadata: &mut Metadata<'_>, name: &[u8]) -> Option<()> {
    let (_, part) = PARTS.iter().find(|(known, _)| *known == name)?;
    metadata.part = *part;
    Some(())
}

fn set_base64(metadata: &mut Metadata<'_>, value: &[u8]) -> Option<()> {
    metadata.base64 = flag(value)?;
    Some(())
}

/// `0` or `1`.
fn flag(value: &[u8]) -> Option<bool> {
    match value {
        b"0" => Some(false),
        b"1" => Some(true),
        _ => None,
    }
}

impl<'a> Metadata<'a> {
    /// The metadata with one `key=value` pair applied, or `None` when the
    /// pair discards the chunk.
    fn apply(mut self, pair: &'a [u8]) -> Option<Self> {
        let (key, value) = split_once(pair, b'=');
        let setter = KEYS.iter().find(|(known, _)| *known == key);
        setter.map_or(Some(()), |(_, set)| {
            set(&mut self, value.unwrap_or_default())
        })?;
        Some(self)
    }

    /// The chunk carrying `payload`, when it is notification data.
    fn chunk(self, payload: &'a [u8]) -> Option<Parsed<'a>> {
        Some(Parsed::Chunk(Chunk {
            id: self.id,
            done: self.done,
            part: self.part?,
            base64: self.base64,
            payload,
        }))
    }
}
