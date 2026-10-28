use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractError(pub String);

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ContractError {}

macro_rules! validated_string {
    ($name:ident, $validate:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = ContractError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                $validate(value)?;
                Ok(Self(value.to_owned()))
            }
        }

        impl TryFrom<String> for $name {
            type Error = ContractError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

validated_string!(SessionId, validate_session_id);
validated_string!(VersionName, validate_version);
validated_string!(Timestamp, validate_timestamp);
validated_string!(MetadataPath, validate_metadata_path);

/// Metadata paths are display strings, read across operating systems. Accept
/// absolute Unix, drive-rooted Windows and UNC paths without interpreting bytes.
fn validate_metadata_path(value: &str) -> Result<(), ContractError> {
    if value.contains('\0') {
        return Err(ContractError("path contains NUL".into()));
    }
    if value.starts_with('/') || windows_absolute(value) {
        return Ok(());
    }
    Err(ContractError("metadata path must be absolute".into()))
}

fn windows_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return matches!(bytes[2], b'/' | b'\\');
    }
    value.strip_prefix("\\\\").is_some_and(|tail| {
        let mut components = tail.split('\\');
        components.next().is_some_and(|part| !part.is_empty())
            && components.next().is_some_and(|part| !part.is_empty())
    })
}

fn validate_session_id(value: &str) -> Result<(), ContractError> {
    let guid =
        uuid::Uuid::parse_str(value).map_err(|_| ContractError("invalid session ID".into()))?;
    if guid.hyphenated().to_string() != value {
        return Err(ContractError(
            "session ID must be a lowercase hyphenated GUID".into(),
        ));
    }
    Ok(())
}

fn validate_version(value: &str) -> Result<(), ContractError> {
    semver::Version::parse(value).map_err(|_| ContractError("invalid SemVer version".into()))?;
    Ok(())
}

fn validate_timestamp(value: &str) -> Result<(), ContractError> {
    if value.len() != 24 || !value.ends_with('Z') || value.as_bytes()[19] != b'.' {
        return Err(ContractError(
            "created_at must be UTC with milliseconds".into(),
        ));
    }
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(|_| ContractError("invalid UTC millisecond timestamp".into()))?;
    Ok(())
}
