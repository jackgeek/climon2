//! The user's `PATH` (docs/windows-distribution.md, "PATH"). The installer
//! reads the stored value of `HKCU\Environment\Path` without expanding it and
//! writes it back as the same registry type, appending the executable home as
//! one literal entry. It records the entry it wrote under
//! `HKCU\Software\climon2`, so an install into another directory replaces it.
//! An entry that already names the home, after expanding variables and
//! ignoring letter case and a trailing backslash, counts as present; if the
//! installer did not write it, it is not recorded. After a change it tells
//! running programs the environment changed.
use std::{
    ffi::OsStr,
    io::{self, Write},
    iter,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
    System::{
        Environment::ExpandEnvironmentStringsW,
        Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ,
            REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE, RegCreateKeyExW, RegDeleteValueW,
            RegQueryValueExW, RegSetValueExW,
        },
    },
    UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    },
};

const ENVIRONMENT: &str = "Environment";
const PATH: &str = "Path";
const RECORD_KEY: &str = r"Software\climon2";
const RECORD: &str = "PathEntry";

/// Adds `home` to the user's `PATH` unless an entry already names it.
pub(super) fn add(home: &Path) -> io::Result<()> {
    let entry = home.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} cannot be written to PATH", home.display()),
        )
    })?;
    let stored = Stored::read()?;
    let edit = Edit::new(&stored.path, stored.recorded.as_deref(), entry, &expand);
    stored.write(&edit)?;
    report(&edit, entry)
}

fn report(edit: &Edit, entry: &str) -> io::Result<()> {
    if !edit.added {
        return Ok(());
    }
    writeln!(
        io::stdout(),
        "climon2: added {entry} to your user PATH; terminals opened before now do not see it"
    )
}

/// The user `PATH` and the installer's record of its entry, as stored.
struct Stored {
    environment: Key,
    records: Key,
    path: String,
    kind: REG_VALUE_TYPE,
    recorded: Option<String>,
}

impl Stored {
    fn read() -> io::Result<Self> {
        let environment = Key::create(ENVIRONMENT)?;
        let records = Key::create(RECORD_KEY)?;
        // A user with no PATH of their own gets the type Windows gives one.
        let (path, kind) = environment
            .text(PATH)?
            .unwrap_or((String::new(), REG_EXPAND_SZ));
        let recorded = records.text(RECORD)?.map(|(entry, _)| entry);
        Ok(Self {
            environment,
            records,
            path,
            kind,
            recorded,
        })
    }

    fn write(&self, edit: &Edit) -> io::Result<()> {
        self.write_path(&edit.path)?;
        self.write_record(&edit.record)
    }

    /// Stores `path` as the same type, if it changed, and says so.
    fn write_path(&self, path: &str) -> io::Result<()> {
        if path == self.path {
            return Ok(());
        }
        self.environment.set(PATH, self.kind, path)?;
        broadcast();
        Ok(())
    }

    /// Stores `record`, if it changed: the entry written, or that there is none.
    fn write_record(&self, record: &Option<String>) -> io::Result<()> {
        if *record == self.recorded {
            return Ok(());
        }
        match record {
            Some(entry) => self.records.set(RECORD, REG_SZ, entry),
            None => self.records.delete(RECORD),
        }
    }
}

/// What an install does to `PATH` and to the record of the entry it wrote.
#[derive(Debug, PartialEq)]
struct Edit {
    path: String,
    record: Option<String>,
    added: bool,
}

impl Edit {
    /// Drops the recorded entry if it names another directory, then appends
    /// `entry` unless an entry names the same directory. The record keeps the
    /// entry the installer wrote, and nothing for an entry it did not.
    fn new(
        path: &str,
        recorded: Option<&str>,
        entry: &str,
        expand: &dyn Fn(&str) -> String,
    ) -> Self {
        let wanted = comparable(entry, expand);
        let stale = recorded.filter(|recorded| comparable(recorded, expand) != wanted);
        let kept: Vec<&str> = path
            .split(';')
            .filter(|item| Some(*item) != stale)
            .collect();
        let path = kept.join(";");
        match kept.iter().find(|item| comparable(item, expand) == wanted) {
            Some(present) => Self {
                record: recorded
                    .filter(|recorded| recorded == present)
                    .map(str::to_owned),
                path,
                added: false,
            },
            None => Self {
                path: appended(&path, entry),
                record: Some(entry.to_owned()),
                added: true,
            },
        }
    }
}

/// An entry as it is compared: expanded, without a trailing backslash, in
/// lower case.
fn comparable(entry: &str, expand: &dyn Fn(&str) -> String) -> String {
    let expanded = expand(entry);
    expanded
        .strip_suffix('\\')
        .unwrap_or(&expanded)
        .to_lowercase()
}

fn appended(path: &str, entry: &str) -> String {
    if path.is_empty() || path.ends_with(';') {
        format!("{path}{entry}")
    } else {
        format!("{path};{entry}")
    }
}

/// `text` with its environment variables expanded as Windows expands `PATH`.
fn expand(text: &str) -> String {
    let source = wide(text);
    // Without a buffer Windows ignores the size and returns the size needed.
    let size = unsafe { ExpandEnvironmentStringsW(source.as_ptr(), ptr::null_mut(), 0u32) };
    let mut expanded = vec![0u16; size as usize];
    unsafe { ExpandEnvironmentStringsW(source.as_ptr(), expanded.as_mut_ptr(), size) };
    String::from_utf16_lossy(&until_nul(expanded.into_iter()))
}

fn until_nul(units: impl Iterator<Item = u16>) -> Vec<u16> {
    units.take_while(|unit| *unit != 0).collect()
}

/// Tells running programs that the user's environment changed. Terminals
/// already open do not reread it.
fn broadcast() {
    let area = wide(ENVIRONMENT);
    // An environment change names its area and no system setting.
    let setting = 0;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            setting,
            area.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            // How long one unresponsive window may hold the install up; only
            // a hung window can tell 5000 from any other wait.
            5_000u32,
            ptr::null_mut(),
        )
    };
}

/// A key under `HKEY_CURRENT_USER`, opened to read and set values. The
/// installer exits soon after, which closes it.
struct Key(HKEY);

impl Key {
    fn create(path: &str) -> io::Result<Self> {
        let mut key = ptr::null_mut();
        check(unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(path).as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            )
        })?;
        Ok(Self(key))
    }

    /// The stored text of the string value `name` and its type, unexpanded,
    /// or `None` if there is no such value.
    fn text(&self, name: &str) -> io::Result<Option<(String, REG_VALUE_TYPE)>> {
        let Some((data, kind)) = self.value(name)? else {
            return Ok(None);
        };
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "the registry value {name} has type {kind}, not a string; it was left unchanged"
                ),
            ));
        }
        decode(&data).map(|text| Some((text, kind)))
    }

    /// The stored bytes of the value `name` and its type, or `None` if there
    /// is no such value.
    fn value(&self, name: &str) -> io::Result<Option<(Vec<u8>, REG_VALUE_TYPE)>> {
        let name = wide(name);
        // Windows overwrites both whatever they start as.
        let mut kind = 0u32;
        let mut size = 0u32;
        let status = self.query(&name, &mut kind, ptr::null_mut(), &mut size);
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(status)?;
        let mut data = vec![0u8; size as usize];
        check(self.query(&name, &mut kind, data.as_mut_ptr(), &mut size))?;
        Ok(Some((data, kind)))
    }

    fn query(&self, name: &[u16], kind: &mut u32, data: *mut u8, size: &mut u32) -> WIN32_ERROR {
        unsafe { RegQueryValueExW(self.0, name.as_ptr(), ptr::null(), kind, data, size) }
    }

    fn set(&self, name: &str, kind: REG_VALUE_TYPE, text: &str) -> io::Result<()> {
        let data = wide(text);
        // Windows refuses any other value.
        let reserved = 0;
        check(unsafe {
            RegSetValueExW(
                self.0,
                wide(name).as_ptr(),
                reserved,
                kind,
                data.as_ptr().cast(),
                (data.len() * size_of::<u16>()) as u32,
            )
        })
    }

    fn delete(&self, name: &str) -> io::Result<()> {
        check(unsafe { RegDeleteValueW(self.0, wide(name).as_ptr()) })
    }
}

/// A string value's UTF-16 bytes as text, up to its terminator.
fn decode(data: &[u8]) -> io::Result<String> {
    let units = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    String::from_utf16(&until_nul(units))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn check(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expands `%HOME%` to `C:\Users\me` and leaves everything else.
    fn expand(text: &str) -> String {
        text.replace("%HOME%", r"C:\Users\me")
    }

    fn edit(path: &str, recorded: Option<&str>) -> Edit {
        Edit::new(path, recorded, r"C:\Users\me\climon2", &expand)
    }

    fn appended(path: &str) -> Edit {
        Edit {
            path: path.to_owned(),
            record: Some(r"C:\Users\me\climon2".to_owned()),
            added: true,
        }
    }

    fn kept(path: &str, record: Option<&str>) -> Edit {
        Edit {
            path: path.to_owned(),
            record: record.map(str::to_owned),
            added: false,
        }
    }

    #[test]
    fn the_home_is_appended_once_as_one_literal_entry() {
        assert_eq!(edit("", None), appended(r"C:\Users\me\climon2"));
        assert_eq!(
            edit("A;%HOME%\\b", None),
            appended(r"A;%HOME%\b;C:\Users\me\climon2")
        );
        assert_eq!(edit("A;", None), appended(r"A;C:\Users\me\climon2"));
        assert_eq!(
            edit(r"A;C:\Users\me\climon2", Some(r"C:\Users\me\climon2")),
            kept(r"A;C:\Users\me\climon2", Some(r"C:\Users\me\climon2"))
        );
    }

    #[test]
    fn an_entry_the_user_wrote_for_the_home_counts_as_present_and_is_not_recorded() {
        for present in [
            r"c:\users\ME\CLIMON2",
            r"C:\Users\me\climon2\",
            r"%HOME%\climon2",
            r"%HOME%\climon2\",
        ] {
            let path = format!("A;{present};B");
            assert_eq!(edit(&path, None), kept(&path, None), "{present}");
        }
    }

    #[test]
    fn a_recorded_entry_for_another_directory_is_replaced() {
        assert_eq!(
            edit(r"A;D:\old;B;D:\old", Some(r"D:\old")),
            appended(r"A;B;C:\Users\me\climon2")
        );
        assert_eq!(
            edit(r"D:\old", Some(r"D:\old")),
            appended(r"C:\Users\me\climon2")
        );
        // Only the literal entry the installer wrote is ever removed.
        assert_eq!(
            edit(r"d:\OLD;D:\old\", Some(r"D:\old")),
            appended(r"d:\OLD;D:\old\;C:\Users\me\climon2")
        );
        assert_eq!(
            edit(r"A;D:\old;%HOME%\climon2", Some(r"D:\old")),
            kept(r"A;%HOME%\climon2", None)
        );
    }

    #[test]
    fn a_record_of_the_home_in_another_spelling_is_not_the_present_entry() {
        assert_eq!(
            edit(r"A;c:\users\me\climon2", Some(r"C:\Users\me\climon2")),
            kept(r"A;c:\users\me\climon2", None)
        );
    }
}
