//! One instance's application input as its library receives it (#109,
//! #111): pure. Input goes in chunks of at most `chunk_bytes`, in order,
//! until the library refuses one. A paste's parts follow one another, with
//! the application's paste markers around them when it had bracketed paste
//! on; once one is refused, the rest of that paste is discarded, and an
//! application that received part of a bracketed paste is owed its end
//! marker (owner decision on #111), sent after each answer, when the
//! library may have room again, and before any later input, until the
//! library accepts it.
use crate::terminal::{Authority, Paste, Request};

const START: &[u8] = b"\x1b[200~";
const END: &[u8] = b"\x1b[201~";

/// Submits one request to the library: accepted, or why not.
pub(super) type Submit<'a> = dyn FnMut(Request) -> Result<(), String> + 'a;

#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct Stream {
    /// How much of the open bracketed paste's start and end markers the
    /// library accepted.
    started: usize,
    ended: usize,
    /// The open paste was refused partway: its later parts are discarded.
    truncated: bool,
    /// What a truncated bracketed paste still owes its application, under
    /// the paste's authority.
    owed: Option<(Authority, Vec<u8>)>,
}

impl Stream {
    /// Submits `request` through `submit`, input in chunks of at most
    /// `chunk` bytes. Returns the refusal to report: refused input carries
    /// every byte not submitted; a refused paste part is reported once, as
    /// itself, and later parts of its paste are discarded silently. Input
    /// waits behind anything a truncated paste owes.
    pub(super) fn submit(
        &mut self,
        request: Request,
        chunk: usize,
        submit: &mut Submit<'_>,
    ) -> Option<(Request, String)> {
        if let Request::Paste(authority, paste) = request {
            return self.paste(authority, paste, chunk, submit);
        }
        match self.settle(chunk, submit) {
            Ok(()) => plain(request, chunk, submit),
            Err(why) => Some((request, why)),
        }
    }

    /// Submits what a truncated paste owes its application, if anything;
    /// whatever the library refuses stays owed.
    pub(super) fn settle(&mut self, chunk: usize, submit: &mut Submit<'_>) -> Result<(), String> {
        let Some((authority, owed)) = self.owed.take() else {
            return Ok(());
        };
        input(authority, &owed, chunk, submit).map_err(|(sent, why)| {
            self.owed = Some((authority, owed[sent..].to_vec()));
            why
        })
    }

    /// One part of a paste: the first begins a paste afresh.
    fn paste(
        &mut self,
        authority: Authority,
        paste: Paste,
        chunk: usize,
        submit: &mut Submit<'_>,
    ) -> Option<(Request, String)> {
        if paste.starts {
            let owed = self.owed.take();
            *self = Self {
                owed,
                ..Self::default()
            };
        }
        if self.truncated {
            return None;
        }
        let refused = self
            .settle(chunk, submit)
            .and_then(|()| self.parts(authority, &paste, chunk, submit));
        let why = refused.err()?;
        self.truncate(authority);
        Some((Request::Paste(authority, paste), why))
    }

    /// The part's start marker, data and end marker, counting what the
    /// library accepts of each marker.
    fn parts(
        &mut self,
        authority: Authority,
        paste: &Paste,
        chunk: usize,
        submit: &mut Submit<'_>,
    ) -> Result<(), String> {
        let (start, end) = markers(paste);
        counted(
            &mut self.started,
            start,
            input(authority, start, chunk, submit),
        )?;
        input(authority, &paste.bytes, chunk, submit).map_err(|(_, why)| why)?;
        counted(&mut self.ended, end, input(authority, end, chunk, submit))
    }

    /// The paste is cut short: once its application received any of its
    /// start marker, it is owed the rest of that marker and the end marker.
    fn truncate(&mut self, authority: Authority) {
        self.truncated = true;
        if self.started > 0 {
            let owed = [&START[self.started..], &END[self.ended..]].concat();
            self.owed = Some((authority, owed));
        }
    }
}

/// The markers around this part: the start marker if it begins a bracketed
/// paste, the end marker if it ends one.
fn markers(paste: &Paste) -> (&'static [u8], &'static [u8]) {
    let marker = |marker: &'static [u8], here: bool| {
        if paste.bracketed && here { marker } else { &[] }
    };
    (marker(START, paste.starts), marker(END, paste.ends))
}

/// Adds what the library accepted of `bytes` to `count`.
fn counted(
    count: &mut usize,
    bytes: &[u8],
    result: Result<(), (usize, String)>,
) -> Result<(), String> {
    let (accepted, result) = match result {
        Ok(()) => (bytes.len(), Ok(())),
        Err((sent, why)) => (sent, Err(why)),
    };
    *count += accepted;
    result
}

/// Any request but a paste: input in chunks, anything else whole.
fn plain(request: Request, chunk: usize, submit: &mut Submit<'_>) -> Option<(Request, String)> {
    match request {
        Request::Input(authority, bytes) => input(authority, &bytes, chunk, submit)
            .err()
            .map(|(sent, why)| (Request::Input(authority, bytes[sent..].to_vec()), why)),
        request => submit(request.clone()).err().map(|why| (request, why)),
    }
}

/// Submits `bytes` as input under `authority`, in chunks of at most `chunk`
/// bytes, in order, until one is refused: then how many bytes were accepted
/// before it, and why.
fn input(
    authority: Authority,
    bytes: &[u8],
    chunk: usize,
    submit: &mut Submit<'_>,
) -> Result<(), (usize, String)> {
    let mut sent = 0;
    for piece in bytes.chunks(chunk) {
        submit(Request::Input(authority, piece.to_vec())).map_err(|why| (sent, why))?;
        sent += piece.len();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELD: Authority = Authority {
        generation: 1,
        period: 2,
        restoration: 3,
    };

    /// A fake library that accepts `room` more chunks, recording them, and
    /// then refuses every chunk with `full` until given more room.
    struct Library {
        room: usize,
        accepted: Vec<Request>,
    }

    impl Library {
        fn new(room: usize) -> Self {
            Self {
                room,
                accepted: Vec::new(),
            }
        }

        fn submit(&mut self, request: Request) -> Result<(), String> {
            if self.room == 0 {
                return Err("full".to_owned());
            }
            self.room -= 1;
            self.accepted.push(request);
            Ok(())
        }

        /// The accepted input's bytes, in order.
        fn bytes(&self) -> Vec<u8> {
            self.accepted
                .iter()
                .flat_map(|request| match request {
                    Request::Input(_, bytes) => bytes.clone(),
                    _ => Vec::new(),
                })
                .collect()
        }
    }

    fn send(
        stream: &mut Stream,
        library: &mut Library,
        request: Request,
    ) -> Option<(Request, String)> {
        stream.submit(request, 4, &mut |request| library.submit(request))
    }

    fn settle(stream: &mut Stream, library: &mut Library) -> Result<(), String> {
        stream.settle(4, &mut |request| library.submit(request))
    }

    fn part(bracketed: bool, starts: bool, bytes: &[u8], ends: bool) -> Request {
        Request::Paste(
            HELD,
            Paste {
                bracketed,
                starts,
                bytes: bytes.to_vec(),
                ends,
            },
        )
    }

    fn input(bytes: &[u8]) -> Request {
        Request::Input(HELD, bytes.to_vec())
    }

    /// COM-03 and COM-04 (pure supplement): input goes in chunks of at most
    /// `chunk_bytes`, at the limit and one past; the first refusal carries
    /// every byte not submitted, and anything else goes whole.
    #[test]
    fn input_goes_in_chunks_and_a_refusal_carries_every_unsent_byte() {
        let mut stream = Stream::default();
        let mut library = Library::new(3);
        assert_eq!(send(&mut stream, &mut library, input(b"abcd")), None);
        assert_eq!(send(&mut stream, &mut library, input(b"efghi")), None);
        assert_eq!(
            library.accepted,
            [input(b"abcd"), input(b"efgh"), input(b"i")]
        );
        assert_eq!(
            send(&mut stream, &mut library, input(b"jk")),
            Some((input(b"jk"), "full".to_owned()))
        );
        let acquire = Some((Request::Acquire, "full".to_owned()));
        assert_eq!(send(&mut stream, &mut library, Request::Acquire), acquire);
        library.room = 2;
        assert_eq!(send(&mut stream, &mut library, Request::Acquire), None);
        assert_eq!(
            send(&mut stream, &mut library, input(b"lmnopqrs")),
            Some((input(b"pqrs"), "full".to_owned()))
        );
        assert_eq!(library.bytes(), b"abcdefghilmno");
    }

    /// INP-03 (pure supplement): a paste reaches a bracketed application
    /// exactly, between its markers, whatever its parts; an unbracketed
    /// one receives the bytes alone.
    #[test]
    fn a_paste_is_exact_between_the_markers_only_when_bracketed() {
        let mut stream = Stream::default();
        let mut library = Library::new(usize::MAX);
        for request in [
            part(true, true, b"ab", false),
            part(true, false, b"cdefg", false),
            part(true, false, b"", true),
            part(false, true, b"xy", false),
            part(false, false, b"z", true),
            part(true, true, b"", true),
        ] {
            assert_eq!(send(&mut stream, &mut library, request), None);
        }
        assert_eq!(
            library.bytes(),
            b"\x1b[200~abcdefg\x1b[201~xyz\x1b[200~\x1b[201~"
        );
        assert_eq!(
            stream,
            Stream {
                started: 6,
                ended: 6,
                ..Stream::default()
            }
        );
    }

    /// INP-03 (Fault; owner decision on #111): a bracketed paste refused
    /// partway is reported once as truncated; its application is owed the
    /// end marker, which goes as soon as the library has room, before any
    /// later input; the rest of the paste is discarded silently, and the
    /// next paste begins afresh.
    #[test]
    fn a_truncated_bracketed_paste_owes_its_end_marker_and_discards_the_rest() {
        let mut stream = Stream::default();
        let mut library = Library::new(3);
        let truncated = part(true, true, b"abcdefgh", false);
        assert_eq!(
            send(&mut stream, &mut library, truncated.clone()),
            Some((truncated, "full".to_owned()))
        );
        assert_eq!(
            send(&mut stream, &mut library, part(true, false, b"ijk", true)),
            None
        );
        assert_eq!(settle(&mut stream, &mut library), Err("full".to_owned()));
        assert_eq!(
            send(&mut stream, &mut library, input(b"z")),
            Some((input(b"z"), "full".to_owned()))
        );
        library.room = 1;
        assert_eq!(settle(&mut stream, &mut library), Err("full".to_owned()));
        library.room = 1;
        assert_eq!(settle(&mut stream, &mut library), Ok(()));
        assert_eq!(settle(&mut stream, &mut library), Ok(()));
        library.room = 6;
        assert_eq!(send(&mut stream, &mut library, input(b"z")), None);
        assert_eq!(
            send(&mut stream, &mut library, part(true, true, b"q", true)),
            None
        );
        assert_eq!(
            library.bytes(),
            b"\x1b[200~abcd\x1b[201~z\x1b[200~q\x1b[201~"
        );
    }

    /// INP-03 (Fault): a truncated paste owes only what its application
    /// lacks: the rest of a start marker cut by the chunk size and the end
    /// marker; the rest of an end marker; nothing when the application
    /// received none of it, or the paste was not bracketed.
    #[test]
    fn a_truncated_paste_owes_only_what_its_application_lacks() {
        for (room, owed) in [(1, &b"0~\x1b[201~"[..]), (4, b"1~"), (0, b"")] {
            let mut stream = Stream::default();
            let mut library = Library::new(room);
            let paste = part(true, true, b"abcd", true);
            assert!(send(&mut stream, &mut library, paste).is_some());
            let received = library.bytes();
            library.room = 3;
            assert_eq!(settle(&mut stream, &mut library), Ok(()));
            assert_eq!(library.bytes(), [&received[..], owed].concat(), "{room}");
        }
        let mut stream = Stream::default();
        let mut library = Library::new(1);
        let plain = part(false, true, b"abcdefgh", true);
        assert!(send(&mut stream, &mut library, plain).is_some());
        assert_eq!(stream.owed, None);
    }

    /// INP-03 (Fault): a new paste waits behind what an earlier truncated
    /// one owes: refused with it while it cannot be paid, it is truncated
    /// itself and owes nothing more.
    #[test]
    fn a_new_paste_waits_behind_what_an_earlier_one_owes() {
        let mut stream = Stream::default();
        let mut library = Library::new(2);
        assert!(send(&mut stream, &mut library, part(true, true, b"abcd", false)).is_some());
        let next = part(true, true, b"x", true);
        assert_eq!(
            send(&mut stream, &mut library, next.clone()),
            Some((next, "full".to_owned()))
        );
        assert_eq!(
            send(&mut stream, &mut library, part(true, false, b"y", true)),
            None
        );
        library.room = 2;
        assert_eq!(settle(&mut stream, &mut library), Ok(()));
        assert_eq!(library.bytes(), b"\x1b[200~\x1b[201~");
    }
}
