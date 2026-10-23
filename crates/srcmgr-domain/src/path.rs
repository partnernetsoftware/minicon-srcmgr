use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;

/// A path kept in the platform's native units, never reconstructed from its
/// display label. Mutation identity uses this value, not the label.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct NativePath(OsString);

/// Lossless, platform-tagged encoding for durable and control transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncodedPath {
    /// Raw bytes of a Unix path name.
    UnixBytes(Vec<u8>),
    /// UTF-16 code units of a Windows path name.
    WindowsUtf16(Vec<u16>),
}

impl EncodedPath {
    /// Stable tag written beside the payload.
    #[must_use]
    pub const fn tag(&self) -> &'static str {
        match self {
            Self::UnixBytes(_) => "unix-bytes",
            Self::WindowsUtf16(_) => "windows-utf16",
        }
    }
}

/// Failure to represent path bytes reported by a tool on this platform.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathError {
    /// The bytes are not valid UTF-8 and this platform has no byte-level path
    /// form to carry them. Nothing is guessed or replaced.
    NotRepresentable,
}

impl NativePath {
    #[must_use]
    pub fn new(value: impl Into<OsString>) -> Self {
        Self(value.into())
    }

    /// Builds a path from bytes that a Git plumbing command printed for it.
    ///
    /// Unix keeps the exact bytes. Windows requires UTF-8, because Git prints
    /// Windows paths that way; anything else is a typed refusal.
    pub fn from_tool_bytes(bytes: &[u8]) -> Result<Self, PathError> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Ok(Self(OsStr::from_bytes(bytes).to_os_string()))
        }
        #[cfg(not(unix))]
        {
            let text = std::str::from_utf8(bytes).map_err(|_| PathError::NotRepresentable)?;
            Ok(Self(OsString::from(text)))
        }
    }

    #[must_use]
    pub fn as_os_str(&self) -> &OsStr {
        &self.0
    }

    /// Restores native units only on the matching platform; never guesses a
    /// path when a profile was copied from another operating system.
    pub fn from_encoded(value: EncodedPath) -> Result<Self, PathError> {
        match value {
            #[cfg(unix)]
            EncodedPath::UnixBytes(bytes) => {
                use std::os::unix::ffi::OsStringExt;
                Ok(Self(OsString::from_vec(bytes)))
            }
            #[cfg(windows)]
            EncodedPath::WindowsUtf16(units) => {
                use std::os::windows::ffi::OsStringExt;
                Ok(Self(OsString::from_wide(&units)))
            }
            #[allow(unreachable_patterns)]
            _ => Err(PathError::NotRepresentable),
        }
    }

    #[must_use]
    pub fn encoded(&self) -> EncodedPath {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            EncodedPath::UnixBytes(self.0.as_bytes().to_vec())
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            EncodedPath::WindowsUtf16(self.0.encode_wide().collect())
        }
    }

    /// A human label with control, bidirectional and invalid units escaped.
    /// It is for display only and never a mutation identity.
    #[must_use]
    pub fn display_label(&self) -> String {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let mut label = String::new();
            for chunk in self.0.as_bytes().utf8_chunks() {
                for character in chunk.valid().chars() {
                    push_escaped(&mut label, character);
                }
                for byte in chunk.invalid() {
                    let _ = write!(label, "\\x{byte:02X}");
                }
            }
            label
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let units: Vec<u16> = self.0.encode_wide().collect();
            let mut label = String::new();
            for decoded in char::decode_utf16(units.iter().copied()) {
                match decoded {
                    Ok(character) => push_escaped(&mut label, character),
                    Err(error) => {
                        let _ = write!(label, "\\u{{{:04X}}}", error.unpaired_surrogate());
                    }
                }
            }
            label
        }
    }
}

fn push_escaped(label: &mut String, character: char) {
    if character.is_control() || is_bidi_control(character) {
        let _ = write!(label, "\\u{{{:04X}}}", u32::from(character));
    } else {
        label.push(character);
    }
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_escapes_controls_and_bidi_but_keeps_text() {
        let path = NativePath::new("docs/a\tb\u{202E}c\u{200F}.md");
        assert_eq!(path.display_label(), "docs/a\\u{0009}b\\u{202E}c\\u{200F}.md");
    }

    #[cfg(unix)]
    #[test]
    fn label_escapes_invalid_utf8_on_unix_without_replacement() {
        let path = NativePath::from_tool_bytes(b"bad\xFFname").expect("unix keeps bytes");
        assert_eq!(path.display_label(), "bad\\xFFname");
        assert_eq!(path.encoded(), EncodedPath::UnixBytes(b"bad\xFFname".to_vec()));
    }

    #[test]
    fn tool_bytes_round_trip_unicode_text() {
        let path = NativePath::from_tool_bytes("中文 space.txt".as_bytes()).expect("utf-8 path");
        assert_eq!(path.as_os_str(), OsStr::new("中文 space.txt"));
        assert_eq!(path.display_label(), "中文 space.txt");
    }
}
