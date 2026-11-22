//! Reads what the Windows dist checks in a shipped PE file: its machine, whether
//! it is a program or a library, the DLLs it imports (delay-loaded ones
//! included) and the names it exports. Plain byte parsing, so its tests run on
//! every platform.

/// The COFF machine of x86-64 code.
pub(crate) const AMD64: u16 = 0x8664;

/// Whether a PE image is a program or a library.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Executable,
    Library,
}

/// The parts of one PE image that the dist checks.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Image {
    pub(crate) machine: u16,
    pub(crate) kind: Kind,
    /// Every imported DLL, delay-loaded ones included, as the file spells it.
    pub(crate) imports: Vec<String>,
    pub(crate) exports: Vec<String>,
}

/// `IMAGE_FILE_EXECUTABLE_IMAGE` and `IMAGE_FILE_DLL` in the COFF characteristics.
const EXECUTABLE_IMAGE: u16 = 0x0002;
const DLL: u16 = 0x2000;
/// The data directories this reader follows.
const EXPORTS: usize = 0;
const IMPORTS: usize = 1;
const DELAY_IMPORTS: usize = 13;
/// Each import descriptor's size and where it holds its DLL name's RVA.
const IMPORT_DESCRIPTOR: (usize, usize) = (20, 12);
const DELAY_DESCRIPTOR: (usize, usize) = (32, 4);

/// The image in `bytes`, or why it is not a PE image this reader understands.
pub(crate) fn parse(bytes: &[u8]) -> Result<Image, String> {
    let file = File::new(bytes)?;
    let mut imports = file.names(IMPORTS, IMPORT_DESCRIPTOR)?;
    imports.extend(file.names(DELAY_IMPORTS, DELAY_DESCRIPTOR)?);
    Ok(Image {
        machine: file.u16(file.coff)?,
        kind: file.kind()?,
        imports,
        exports: file.exports()?,
    })
}

/// A PE file's bytes and where its headers start.
struct File<'a> {
    bytes: &'a [u8],
    coff: usize,
    optional: usize,
}

impl<'a> File<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, String> {
        if !bytes.starts_with(b"MZ") {
            return Err("no MZ header".to_owned());
        }
        let file = Self {
            bytes,
            coff: 0,
            optional: 0,
        };
        let signature = file.u32(0x3c)? as usize;
        if file.slice(signature, 4)? != b"PE\0\0" {
            return Err("no PE signature".to_owned());
        }
        Ok(Self {
            coff: signature + 4,
            optional: signature + 24,
            ..file
        })
    }

    fn slice(&self, at: usize, length: usize) -> Result<&'a [u8], String> {
        at.checked_add(length)
            .and_then(|end| self.bytes.get(at..end))
            .ok_or_else(|| "truncated PE file".to_owned())
    }

    fn u16(&self, at: usize) -> Result<u16, String> {
        let bytes = self.slice(at, 2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&self, at: usize) -> Result<u32, String> {
        let bytes = self.slice(at, 4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn kind(&self) -> Result<Kind, String> {
        let characteristics = self.u16(self.coff + 18)?;
        if characteristics & DLL != 0 {
            Ok(Kind::Library)
        } else if characteristics & EXECUTABLE_IMAGE != 0 {
            Ok(Kind::Executable)
        } else {
            Err("not an executable image".to_owned())
        }
    }

    /// The RVA of data directory `index`, or `None` when the image has none.
    fn directory(&self, index: usize) -> Result<Option<u32>, String> {
        // PE32 and PE32+ optional headers differ in where the directories start.
        let start = match self.u16(self.optional)? {
            0x10b => 96,
            0x20b => 112,
            magic => return Err(format!("unknown optional header magic {magic:#x}")),
        };
        if index >= self.u32(self.optional + start - 4)? as usize {
            return Ok(None);
        }
        let rva = self.u32(self.optional + start + index * 8)?;
        Ok((rva != 0).then_some(rva))
    }

    /// The file offset of `rva`, through the section that holds it.
    fn offset(&self, rva: u32) -> Result<usize, String> {
        let sections = self.u16(self.coff + 2)? as usize;
        let table = self.optional + self.u16(self.coff + 16)? as usize;
        for section in (0..sections).map(|index| table + index * 40) {
            let start = self.u32(section + 12)?;
            let length = self.u32(section + 8)?.max(self.u32(section + 16)?);
            if (start..start.saturating_add(length)).contains(&rva) {
                return Ok((rva - start + self.u32(section + 20)?) as usize);
            }
        }
        Err(format!("RVA {rva:#x} is in no section"))
    }

    /// The NUL-terminated string at `rva`.
    fn string(&self, rva: u32) -> Result<String, String> {
        let start = self.offset(rva)?;
        let rest = self.slice(start, 0).map(|_| &self.bytes[start..])?;
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| "truncated PE file".to_owned())?;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }

    /// The DLL names of the descriptor table in directory `index`, each
    /// descriptor `(size, name field)`, up to the one whose name is zero.
    fn names(&self, index: usize, (size, field): (usize, usize)) -> Result<Vec<String>, String> {
        let Some(table) = self.directory(index)? else {
            return Ok(Vec::new());
        };
        let table = self.offset(table)?;
        let mut names = Vec::new();
        for descriptor in (0..).map(|entry| table + entry * size) {
            match self.u32(descriptor + field)? {
                0 => break,
                name => names.push(self.string(name)?),
            }
        }
        Ok(names)
    }

    fn exports(&self) -> Result<Vec<String>, String> {
        let Some(directory) = self.directory(EXPORTS)? else {
            return Ok(Vec::new());
        };
        let directory = self.offset(directory)?;
        let count = self.u32(directory + 24)? as usize;
        let table = self.offset(self.u32(directory + 32)?)?;
        (0..count)
            .map(|entry| self.string(self.u32(table + entry * 4)?))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PE32+ image with one section holding the import, delay-import and
    /// export tables it is given.
    struct Builder {
        machine: u16,
        characteristics: u16,
        imports: Vec<&'static str>,
        delayed: Vec<&'static str>,
        exports: Vec<&'static str>,
    }

    const SECTION_RVA: u32 = 0x1000;
    const SECTION_OFFSET: usize = 0x200;
    const EXECUTABLE_IMAGE: u16 = 0x0002;
    const DLL: u16 = 0x2000;

    impl Builder {
        fn program() -> Self {
            Self {
                machine: AMD64,
                characteristics: EXECUTABLE_IMAGE,
                imports: vec!["KERNEL32.dll"],
                delayed: Vec::new(),
                exports: Vec::new(),
            }
        }

        fn build(&self) -> Vec<u8> {
            let mut section = Vec::new();
            let rva = |section: &Vec<u8>| SECTION_RVA + section.len() as u32;
            let string = |section: &mut Vec<u8>, text: &str| {
                let at = rva(section);
                section.extend_from_slice(text.as_bytes());
                section.push(0);
                at
            };
            let names: Vec<_> = self
                .imports
                .iter()
                .map(|name| string(&mut section, name))
                .collect();
            let delayed: Vec<_> = self
                .delayed
                .iter()
                .map(|name| string(&mut section, name))
                .collect();
            let exported: Vec<_> = self
                .exports
                .iter()
                .map(|name| string(&mut section, name))
                .collect();
            let import_table = rva(&section);
            for name in &names {
                let mut descriptor = [0u8; 20];
                descriptor[12..16].copy_from_slice(&name.to_le_bytes());
                section.extend_from_slice(&descriptor);
            }
            section.extend_from_slice(&[0; 20]);
            let delay_table = rva(&section);
            for name in &delayed {
                let mut descriptor = [0u8; 32];
                descriptor[0..4].copy_from_slice(&1u32.to_le_bytes());
                descriptor[4..8].copy_from_slice(&name.to_le_bytes());
                section.extend_from_slice(&descriptor);
            }
            section.extend_from_slice(&[0; 32]);
            let name_table = rva(&section);
            for name in &exported {
                section.extend_from_slice(&name.to_le_bytes());
            }
            let export_table = rva(&section);
            let mut directory = [0u8; 40];
            directory[24..28].copy_from_slice(&(exported.len() as u32).to_le_bytes());
            directory[32..36].copy_from_slice(&name_table.to_le_bytes());
            section.extend_from_slice(&directory);

            let mut image = vec![0u8; SECTION_OFFSET];
            image[0..2].copy_from_slice(b"MZ");
            image[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
            image[0x40..0x44].copy_from_slice(b"PE\0\0");
            let coff = 0x44;
            image[coff..coff + 2].copy_from_slice(&self.machine.to_le_bytes());
            image[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
            image[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
            image[coff + 18..coff + 20].copy_from_slice(&self.characteristics.to_le_bytes());
            let optional = coff + 20;
            image[optional..optional + 2].copy_from_slice(&0x20bu16.to_le_bytes());
            image[optional + 108..optional + 112].copy_from_slice(&16u32.to_le_bytes());
            let mut directory = |index: usize, rva: u32, size: u32| {
                let at = optional + 112 + index * 8;
                image[at..at + 4].copy_from_slice(&rva.to_le_bytes());
                image[at + 4..at + 8].copy_from_slice(&size.to_le_bytes());
            };
            directory(0, export_table, 40);
            directory(1, import_table, 20 * (names.len() as u32 + 1));
            if !delayed.is_empty() {
                directory(13, delay_table, 32 * (delayed.len() as u32 + 1));
            }
            let header = optional + 240;
            let size = section.len() as u32;
            image[header..header + 8].copy_from_slice(b".rdata\0\0");
            image[header + 8..header + 12].copy_from_slice(&size.to_le_bytes());
            image[header + 12..header + 16].copy_from_slice(&SECTION_RVA.to_le_bytes());
            image[header + 16..header + 20].copy_from_slice(&size.to_le_bytes());
            image[header + 20..header + 24].copy_from_slice(&(SECTION_OFFSET as u32).to_le_bytes());
            image.extend_from_slice(&section);
            image
        }
    }

    #[test]
    fn a_program_reports_its_machine_kind_and_imports() {
        let image = parse(&Builder::program().build()).unwrap();
        assert_eq!(
            image,
            Image {
                machine: 0x8664,
                kind: Kind::Executable,
                imports: vec!["KERNEL32.dll".to_owned()],
                exports: Vec::new(),
            }
        );
    }

    #[test]
    fn a_library_reports_its_exports_and_delay_loaded_imports() {
        let image = parse(
            &Builder {
                characteristics: EXECUTABLE_IMAGE | DLL,
                imports: vec!["KERNEL32.dll", "api-ms-win-crt-heap-l1-1-0.dll"],
                delayed: vec!["icu.dll"],
                exports: vec!["climon2_client_get_interface"],
                ..Builder::program()
            }
            .build(),
        )
        .unwrap();
        assert_eq!(image.kind, Kind::Library);
        assert_eq!(
            image.imports,
            ["KERNEL32.dll", "api-ms-win-crt-heap-l1-1-0.dll", "icu.dll"]
        );
        assert_eq!(image.exports, ["climon2_client_get_interface"]);
    }

    #[test]
    fn another_machine_is_reported_as_it_is() {
        let image = parse(
            &Builder {
                machine: 0x014c,
                ..Builder::program()
            }
            .build(),
        )
        .unwrap();
        assert_eq!(image.machine, 0x014c);
    }

    #[test]
    fn an_object_that_is_no_image_is_refused() {
        let error = parse(
            &Builder {
                characteristics: 0,
                ..Builder::program()
            }
            .build(),
        )
        .unwrap_err();
        assert_eq!(error, "not an executable image");
    }

    #[test]
    fn text_a_truncated_image_and_a_table_outside_every_section_are_refused() {
        assert_eq!(parse(b"#!/bin/sh\n").unwrap_err(), "no MZ header");
        let image = Builder::program().build();
        assert_eq!(parse(&image[..0x60]).unwrap_err(), "truncated PE file");
        assert_eq!(
            parse(&image[..SECTION_OFFSET + 4]).unwrap_err(),
            "truncated PE file"
        );
        let mut elsewhere = image.clone();
        let import_rva = 0x44 + 20 + 112 + 8;
        elsewhere[import_rva..import_rva + 4].copy_from_slice(&0x9000u32.to_le_bytes());
        assert_eq!(
            parse(&elsewhere).unwrap_err(),
            "RVA 0x9000 is in no section"
        );
        let mut no_pe = image;
        no_pe[0x40] = b'X';
        assert_eq!(parse(&no_pe).unwrap_err(), "no PE signature");
    }
}
