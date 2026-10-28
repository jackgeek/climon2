use serde::de::DeserializeOwned;

pub(crate) fn from_object<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    let first = bytes.iter().find(|byte| !byte.is_ascii_whitespace());
    if first != Some(&b'{') {
        return Err(serde::de::Error::custom("expected a JSON object"));
    }
    serde_json::from_slice(bytes)
}
