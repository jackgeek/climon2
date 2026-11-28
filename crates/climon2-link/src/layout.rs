//! The one mechanical rule that lays out every record body (#480): fields
//! in order, integers big-endian, a byte string or a list behind a `u32`
//! length or count, no padding and no optional trailing field. A struct
//! declared with [`layout!`] is laid out as its fields in the order written,
//! and an enum declared with [`tagged!`] as its tag, then its active
//! variant's fields.
//!
//! Reading checks only the layout. A body that ends early, holds bytes past
//! its layout, gives a tag no variant has, or holds `text` that is not
//! UTF-8 reads as nothing. What a value means is its reader's business.

/// What is left to read of one body.
pub(crate) struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    /// The next `size` bytes, when there are that many.
    pub(crate) fn bytes(&mut self, size: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.0.split_at_checked(size)?;
        self.0 = rest;
        Some(taken)
    }

    /// The next `N` bytes, when there are that many.
    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (taken, rest) = self.0.split_first_chunk()?;
        self.0 = rest;
        Some(*taken)
    }
}

/// A value with a place in a record body.
pub(crate) trait Field: Sized {
    /// Appends the value's bytes to `out`.
    fn put(&self, out: &mut Vec<u8>);

    /// Reads the next value, or nothing when the bytes break its layout.
    fn take(reader: &mut Reader<'_>) -> Option<Self>;
}

/// The value a whole body holds, or nothing when the body breaks the
/// value's layout or runs past it.
pub(crate) fn decode<T>(body: &[u8], take: impl FnOnce(&mut Reader<'_>) -> Option<T>) -> Option<T> {
    let mut reader = Reader(body);
    let value = take(&mut reader)?;
    reader.0.is_empty().then_some(value)
}

/// A value whose bytes start with a tag that says how the rest is laid out.
pub(crate) trait Tagged: Sized {
    /// The tag's own field.
    type Tag: Field;

    /// The value's tag.
    fn tag(&self) -> Self::Tag;

    /// Appends the bytes behind the tag.
    fn put_payload(&self, out: &mut Vec<u8>);

    /// Reads the bytes behind `tag`, or nothing when no variant has that
    /// tag or the bytes break its layout.
    fn take_payload(tag: Self::Tag, reader: &mut Reader<'_>) -> Option<Self>;
}

/// The record that carries `value`: its tag, then its body.
pub(crate) fn record<T: Tagged<Tag = u8>>(value: &T) -> Vec<u8> {
    let mut record = vec![value.tag()];
    value.put_payload(&mut record);
    record
}

/// The value that `record`, tag included, carries, or nothing when the
/// record breaks the layout its tag selects.
pub(crate) fn from_record<T: Tagged<Tag = u8>>(record: &[u8]) -> Option<T> {
    let (&tag, body) = record.split_first()?;
    decode(body, |reader| T::take_payload(tag, reader))
}

impl<const N: usize> Field for [u8; N] {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend(self);
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        reader.array()
    }
}

impl Field for u8 {
    fn put(&self, out: &mut Vec<u8>) {
        out.push(*self);
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        reader.array().map(u8::from_be_bytes)
    }
}

impl Field for u16 {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend(self.to_be_bytes());
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        reader.array().map(u16::from_be_bytes)
    }
}

impl Field for u32 {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend(self.to_be_bytes());
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        reader.array().map(u32::from_be_bytes)
    }
}

impl Field for u64 {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend(self.to_be_bytes());
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        reader.array().map(u64::from_be_bytes)
    }
}

/// `bytes`: a `u32` length, then that many bytes.
impl Field for Vec<u8> {
    fn put(&self, out: &mut Vec<u8>) {
        length(self.len()).put(out);
        out.extend(self);
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        let size = u32::take(reader)?;
        reader.bytes(size as usize).map(<[u8]>::to_vec)
    }
}

/// `text`: `bytes` holding valid UTF-8.
impl Field for String {
    fn put(&self, out: &mut Vec<u8>) {
        self.as_bytes().to_vec().put(out);
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        String::from_utf8(Vec::take(reader)?).ok()
    }
}

/// `tokens`: a `u32` count, then that many `text` values.
impl Field for Vec<String> {
    fn put(&self, out: &mut Vec<u8>) {
        put_list(self, out);
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        take_list(reader)
    }
}

/// A `u32` count, then that many `bytes` values.
impl Field for Vec<Vec<u8>> {
    fn put(&self, out: &mut Vec<u8>) {
        put_list(self, out);
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        take_list(reader)
    }
}

/// A length or count as its `u32` field. Nothing the link code holds is
/// longer: every record fits its receiver's `u32` ceiling.
fn length(size: usize) -> u32 {
    u32::try_from(size).expect("a record is shorter than 4 GiB")
}

fn put_list<T: Field>(items: &[T], out: &mut Vec<u8>) {
    length(items.len()).put(out);
    items.iter().for_each(|item| item.put(out));
}

/// A `u32` count, then that many values. Each value holds at least four
/// bytes, so a count larger than the body stops at the body's end and
/// nothing is held for the values it never had.
fn take_list<T: Field>(reader: &mut Reader<'_>) -> Option<Vec<T>> {
    let count = u32::take(reader)?;
    (0..count).map(|_| T::take(reader)).collect()
}

/// Declares a struct laid out as its fields in the order written.
macro_rules! layout {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $($(#[$field_meta:meta])* $field_vis:vis $field:ident: $type:ty,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq)]
        $vis struct $name {
            $($(#[$field_meta])* $field_vis $field: $type,)*
        }

        impl $crate::layout::Field for $name {
            fn put(&self, out: &mut Vec<u8>) {
                $($crate::layout::Field::put(&self.$field, out);)*
            }

            fn take(reader: &mut $crate::layout::Reader<'_>) -> Option<Self> {
                Some(Self {
                    $($field: $crate::layout::Field::take(reader)?,)*
                })
            }
        }
    };
}
pub(crate) use layout;

/// Declares an enum laid out as its tag, then its active variant's fields.
/// Each variant before the `;` has one tag and at most one field. Each
/// after it takes every tag its pattern matches, and its field is itself
/// [`Tagged`] with the same tag: it lays out its own payload.
macro_rules! tagged {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident: $tag:ty {
            $($(#[$variant_meta:meta])* $variant:ident $(($payload:ty))? = $value:literal,)*
            $(; $($(#[$inner_meta:meta])* $inner:ident($inner_type:ty) in $range:pat,)*)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq)]
        $vis enum $name {
            $($(#[$variant_meta])* $variant $(($payload))?,)*
            $($($(#[$inner_meta])* $inner($inner_type),)*)?
        }

        impl $crate::layout::Tagged for $name {
            type Tag = $tag;

            fn tag(&self) -> $tag {
                match self {
                    $(Self::$variant { .. } => $value,)*
                    $($(Self::$inner(inner) => $crate::layout::Tagged::tag(inner),)*)?
                }
            }

            fn put_payload(&self, out: &mut Vec<u8>) {
                match self {
                    $(Self::$variant $(($crate::layout::tagged!(@bind field $payload)))? => {
                        $(<$payload as $crate::layout::Field>::put(field, out);)?
                    })*
                    $($(Self::$inner(inner) => $crate::layout::Tagged::put_payload(inner, out),)*)?
                }
            }

            fn take_payload(tag: $tag, reader: &mut $crate::layout::Reader<'_>) -> Option<Self> {
                let _ = &reader;
                match tag {
                    $($value => Some(Self::$variant $((<$payload as $crate::layout::Field>::take(reader)?))?),)*
                    $($($range => Some(Self::$inner(<$inner_type as $crate::layout::Tagged>::take_payload(tag, reader)?)),)*)?
                    _ => None,
                }
            }
        }
    };
    (@bind $binding:ident $payload:ty) => {
        $binding
    };
}
pub(crate) use tagged;
