//! Bounded navigation metadata, never authorization or cached repository data.
use srcmgr_domain::{EncodedPath, NativePath};
use crate::{MainRequest, Pane};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Navigation {
    pub focus: Pane,
    pub source: Pane,
    pub file: Option<NativePath>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub stash: Option<String>,
    pub stash_mode: bool,
    pub cursor: usize,
    pub scroll: usize,
    pub request: Option<MainRequest>,
}

impl Navigation {
    pub fn encode(&self) -> String {
        let fields = ["1".into(), pane(self.focus).into(), pane(self.source).into(), self.file.as_ref().map(path).unwrap_or_default(),
            self.branch.as_deref().map(hex).unwrap_or_default(), self.commit.as_deref().map(hex).unwrap_or_default(),
            self.stash.as_deref().map(hex).unwrap_or_default(), u8::from(self.stash_mode).to_string(), self.cursor.to_string(), self.scroll.to_string(),
            self.request.as_ref().map(request).unwrap_or_default()];
        fields.join("\n")
    }

    pub fn decode(value: &str) -> Result<Self, &'static str> {
        if value.len() > 256 * 1024 { return Err("Navigation metadata exceeds its limit; stored record retained"); }
        let f: Vec<_> = value.split('\n').collect();
        if f.len() != 11 || f[0] != "1" { return Err("Unsupported navigation record; stored record retained"); }
        let optional_text = |value: &str| if value.is_empty() { Ok(None) } else { unhex(value).map(Some) };
        let index = |value: &str| value.parse::<usize>().ok().filter(|n| *n <= 10_000_000).ok_or("Invalid navigation offset; stored record retained");
        Ok(Self { focus: read_pane(f[1])?, source: read_pane(f[2])?, file: if f[3].is_empty() { None } else { Some(read_path(f[3])?) },
            branch: optional_text(f[4])?, commit: optional_text(f[5])?, stash: optional_text(f[6])?,
            stash_mode: match f[7] { "0" => false, "1" => true, _ => return Err("Invalid navigation flag") },
            cursor: index(f[8])?, scroll: index(f[9])?, request: if f[10].is_empty() { None } else { Some(read_request(f[10])?) } })
    }
}

fn pane(value: Pane) -> &'static str { match value { Pane::Status => "status", Pane::Files => "files", Pane::Branches => "branches", Pane::Commits => "commits", Pane::Stashes => "stashes", Pane::Main => "main" } }
fn read_pane(value: &str) -> Result<Pane, &'static str> { match value { "status" => Ok(Pane::Status), "files" => Ok(Pane::Files), "branches" => Ok(Pane::Branches), "commits" => Ok(Pane::Commits), "stashes" => Ok(Pane::Stashes), "main" => Ok(Pane::Main), _ => Err("Invalid navigation pane") } }
fn hex(value: &str) -> String { bytes_hex(value.as_bytes()) }
fn bytes_hex(value: &[u8]) -> String { value.iter().map(|byte| format!("{byte:02x}")).collect() }
fn bytes(value: &str) -> Result<Vec<u8>, &'static str> {
    if !value.len().is_multiple_of(2) || value.len() > 128 * 1024 || !value.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("Invalid navigation text encoding"); }
    (0..value.len()).step_by(2).map(|i| u8::from_str_radix(&value[i..i+2], 16).map_err(|_| "Invalid navigation text encoding")).collect()
}
fn unhex(value: &str) -> Result<String, &'static str> { String::from_utf8(bytes(value)?).map_err(|_| "Invalid navigation text") }
fn path(value: &NativePath) -> String {
    match value.encoded() { EncodedPath::UnixBytes(v) => format!("u:{}", bytes_hex(&v)), EncodedPath::WindowsUtf16(v) => format!("w:{}", bytes_hex(&v.iter().flat_map(|u| u.to_le_bytes()).collect::<Vec<_>>())) }
}
fn read_path(value: &str) -> Result<NativePath, &'static str> {
    let (tag, payload) = value.split_once(':').ok_or("Invalid navigation path")?;
    let payload = bytes(payload)?;
    let encoded = match tag { "u" => EncodedPath::UnixBytes(payload), "w" if payload.len().is_multiple_of(2) => EncodedPath::WindowsUtf16(payload.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect()), _ => return Err("Invalid navigation path tag") };
    NativePath::from_encoded(encoded).map_err(|_| "Navigation path belongs to another platform; record retained")
}
fn request(value: &MainRequest) -> String {
    match value {
        MainRequest::File { path: p, staged_view, untracked } => format!("file\t{}\t{}\t{}", path(p), u8::from(*staged_view), u8::from(*untracked)),
        MainRequest::Commit { id } => format!("commit\t{}", hex(id)),
        MainRequest::CommitFiles { id } => format!("files\t{}", hex(id)),
        MainRequest::Stash { id } => format!("stash\t{}", hex(id)),
        MainRequest::CommitFile { id, file } => format!("detail\t{}\t{}\t{}\t{}\t{}\t{}", hex(id), hex(&file.status), hex(&file.old_mode), hex(&file.new_mode), path(&file.path), file.old_path.as_ref().map(path).unwrap_or_default()),
    }
}
fn read_request(value: &str) -> Result<MainRequest, &'static str> {
    let f: Vec<_> = value.split('\t').collect();
    match f.as_slice() {
        ["file", p, staged, untracked] => { let flag = |v: &str| match v { "0" => Ok(false), "1" => Ok(true), _ => Err("Invalid navigation flag") }; Ok(MainRequest::File { path: read_path(p)?, staged_view: flag(staged)?, untracked: flag(untracked)? }) }
        ["commit", id] => Ok(MainRequest::Commit { id: unhex(id)? }),
        ["files", id] => Ok(MainRequest::CommitFiles { id: unhex(id)? }),
        ["stash", id] => Ok(MainRequest::Stash { id: unhex(id)? }),
        ["detail", id, status, old_mode, new_mode, p, old] => Ok(MainRequest::CommitFile { id: unhex(id)?, file: crate::CommittedFile { status: unhex(status)?, old_mode: unhex(old_mode)?, new_mode: unhex(new_mode)?, path: read_path(p)?, old_path: if old.is_empty() { None } else { Some(read_path(old)?) } } }),
        _ => Err("Invalid navigation detail request"),
    }
}
