//! Reading and writing a text file without changing how it is encoded (PRD 08).
//!
//! A conflict result is written back into a file that already existed. Its
//! bytes, its encoding and its newline style belong to the user, so this module
//! reads them, reports them, and writes them back exactly. Three encodings are
//! supported and everything else is refused by name rather than re-encoded on a
//! guess: a silent conversion is corruption with good intentions.

use std::path::Path;

/// The encodings this slice reads and writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
}

/// The editor's bound, owned by the buffer so the two cannot drift apart.
pub use srcmgr_ui::{MAX_BYTES, MAX_LINES};

/// The newline style the file used, as found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Newline {
    Lf,
    Crlf,
    /// Both styles appear; the file is written back with what the text holds.
    Mixed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextFile {
    pub encoding: Encoding,
    pub newline: Newline,
    pub text: String,
}

/// Why a file could not be read as text this slice can write back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextError {
    /// The bytes are not UTF-8, UTF-16LE with a BOM, or UTF-8 with a BOM.
    Undecodable,
    /// The file could not be read or written.
    Io,
}

impl TextError {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Undecodable => {
                "the file is not UTF-8, UTF-8 with a BOM or UTF-16LE with a BOM; this version will not re-encode it"
            }
            Self::Io => "the file could not be read or written",
        }
    }
}

fn decode_utf16le(bytes: &[u8]) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

/// Whether the file fits the editor's bound. Both limits are checked, because a
/// file can be small in bytes and enormous in lines.
pub fn within_bound(file: &TextFile) -> bool {
    file.text.len() <= MAX_BYTES && file.text.lines().count() <= MAX_LINES
}

/// Rewrites the text to the newline style the file already used. A file with
/// both styles is left exactly as the editor produced it, because normalising
/// it would change lines the user did not touch.
pub fn normalise_newlines(file: &mut TextFile) {
    let unified = file.text.replace("\r\n", "\n");
    file.text = match file.newline {
        Newline::Lf => unified,
        Newline::Crlf => unified.replace('\n', "\r\n"),
        Newline::Mixed => return,
    };
}

/// Counts lines that begin with a conflict marker. This is a warning, never a
/// refusal: the index decides whether a path is resolved.
pub fn marker_lines(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            ["<<<<<<<", "=======", ">>>>>>>", "|||||||"]
                .iter()
                .any(|marker| line.starts_with(marker))
        })
        .count()
}

/// Reads a file as text and records how it was stored.
pub fn read(path: &Path) -> Result<TextFile, TextError> {
    let bytes = std::fs::read(path).map_err(|_| TextError::Io)?;
    let (encoding, body) = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (Encoding::Utf8Bom, &bytes[3..])
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        (Encoding::Utf16Le, &bytes[2..])
    } else {
        (Encoding::Utf8, &bytes[..])
    };
    let text = match encoding {
        Encoding::Utf16Le => decode_utf16le(body).ok_or(TextError::Undecodable)?,
        _ => std::str::from_utf8(body).map_err(|_| TextError::Undecodable)?.to_owned(),
    };
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    let newline = match (crlf, lf) {
        (0, _) => Newline::Lf,
        (_, 0) => Newline::Crlf,
        _ => Newline::Mixed,
    };
    Ok(TextFile { encoding, newline, text })
}

/// Encodes the text exactly as the file recorded it.
pub fn encode(file: &TextFile) -> Vec<u8> {
    match file.encoding {
        Encoding::Utf8 => file.text.as_bytes().to_vec(),
        Encoding::Utf8Bom => {
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(file.text.as_bytes());
            bytes
        }
        Encoding::Utf16Le => {
            let mut bytes = vec![0xFF, 0xFE];
            for unit in file.text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
    }
}

/// Writes the file beside itself and renames it, so a failed write leaves the
/// original bytes untouched.
pub fn write(path: &Path, file: &TextFile) -> Result<(), TextError> {
    let temporary = path.with_extension("srcmgr-tmp");
    std::fs::write(&temporary, encode(file)).map_err(|_| TextError::Io)?;
    std::fs::rename(&temporary, path).map_err(|_| {
        let _ = std::fs::remove_file(&temporary);
        TextError::Io
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("srcmgr-text-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root.join("a.txt")
    }

    #[test]
    fn plain_utf8_with_lf_round_trips_byte_for_byte() {
        let path = scratch("utf8");
        std::fs::write(&path, b"one\ntwo\n").unwrap();
        let file = read(&path).unwrap();
        assert_eq!(file.encoding, Encoding::Utf8);
        assert_eq!(file.newline, Newline::Lf);
        assert_eq!(file.text, "one\ntwo\n");
        write(&path, &TextFile { text: "one\nthree\n".into(), ..file }).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"one\nthree\n");
    }

    #[test]
    fn a_utf8_bom_is_kept_and_not_turned_into_text() {
        let path = scratch("bom");
        let mut original = vec![0xEF, 0xBB, 0xBF];
        original.extend_from_slice(b"one\r\ntwo\r\n");
        std::fs::write(&path, &original).unwrap();
        let file = read(&path).unwrap();
        assert_eq!(file.encoding, Encoding::Utf8Bom);
        assert_eq!(file.newline, Newline::Crlf);
        assert_eq!(file.text, "one\r\ntwo\r\n");
        write(&path, &TextFile { text: "one\r\nthree\r\n".into(), ..file }).unwrap();
        let written = std::fs::read(&path).unwrap();
        assert!(written.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(&written[3..], b"one\r\nthree\r\n");
    }

    #[test]
    fn utf16le_with_a_bom_round_trips_through_the_same_encoding() {
        let path = scratch("utf16");
        let text = "one\r\ntwo\r\n";
        let mut original = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            original.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(&path, &original).unwrap();
        let file = read(&path).unwrap();
        assert_eq!(file.encoding, Encoding::Utf16Le);
        assert_eq!(file.text, text);
        write(&path, &TextFile { text: "one\r\nthree\r\n".into(), ..file }).unwrap();
        let written = std::fs::read(&path).unwrap();
        assert_eq!(&written[..2], &[0xFF, 0xFE]);
        let units: Vec<u16> = written[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), "one\r\nthree\r\n");
    }

    #[test]
    fn bytes_that_are_not_decodable_are_refused_rather_than_guessed() {
        let path = scratch("binary");
        std::fs::write(&path, [0x00, 0xFF, 0xFE, 0x00, 0x41]).unwrap();
        assert_eq!(read(&path), Err(TextError::Undecodable));
        assert!(read(&path).unwrap_err().reason().contains("will not re-encode"));
    }

    #[test]
    fn a_missing_file_is_an_io_error_and_writes_nothing() {
        let path = scratch("missing").with_extension("absent");
        assert_eq!(read(&path), Err(TextError::Io));
        assert!(!path.exists());
    }


    #[test]
    fn the_bound_is_checked_in_bytes_and_in_lines() {
        let path = scratch("bound");
        std::fs::write(&path, "one\n").unwrap();
        let file = read(&path).unwrap();
        assert!(within_bound(&file));
        let too_long = TextFile { text: "x\n".repeat(MAX_LINES + 1), ..file.clone() };
        assert!(!within_bound(&too_long));
        let too_big = TextFile { text: "x".repeat(MAX_BYTES + 1), ..file };
        assert!(!within_bound(&too_big));
    }

    #[test]
    fn a_lf_file_stays_lf_and_a_crlf_file_gets_crlf_back() {
        let path = scratch("newline");
        std::fs::write(&path, "one\r\ntwo\r\n").unwrap();
        let mut file = read(&path).unwrap();
        assert_eq!(file.newline, Newline::Crlf);
        file.text = "one\nmain\nfeature\n".into();
        normalise_newlines(&mut file);
        assert_eq!(file.text, "one\r\nmain\r\nfeature\r\n");
        write(&path, &file).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"one\r\nmain\r\nfeature\r\n");

        let path = scratch("lf");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let mut file = read(&path).unwrap();
        assert_eq!(file.newline, Newline::Lf);
        file.text = "one\r\nmain\n".into();
        normalise_newlines(&mut file);
        assert_eq!(file.text, "one\nmain\n");

        // A file with both styles is not rewritten behind the user's back.
        let path = scratch("mixed");
        std::fs::write(&path, "one\r\ntwo\n").unwrap();
        let mut file = read(&path).unwrap();
        assert_eq!(file.newline, Newline::Mixed);
        file.text = "one\r\nthree\n".into();
        normalise_newlines(&mut file);
        assert_eq!(file.text, "one\r\nthree\n");
    }

    #[test]
    fn conflict_markers_are_counted_as_a_warning() {
        assert_eq!(marker_lines("one\ntwo\n"), 0);
        assert_eq!(marker_lines("<<<<<<< HEAD\none\n=======\ntwo\n>>>>>>> feature\n"), 3);
        assert_eq!(marker_lines("||||||| base\n"), 1);
        assert_eq!(marker_lines("  <<<<<<< indented is text\n"), 0);
    }
}