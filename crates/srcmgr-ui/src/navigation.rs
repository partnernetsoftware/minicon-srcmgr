//! Bounded navigation metadata, never authorization or cached repository data.
use srcmgr_domain::{EncodedPath, NativePath};
use crate::{MainRequest, Pane};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Navigation {
    pub back: Vec<NavigationFrame>,
    pub detail_file: Option<NativePath>,
    pub queries: [String; 3],
    pub anchors: [Option<NavigationSelection>; 3],
    pub anchor_positions: [(usize, usize); 3],
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavigationSelection { File(NativePath), Branch(String), Commit(String) }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationFrame {
    pub request: MainRequest,
    pub cursor: usize,
    pub scroll: usize,
    pub file: Option<NativePath>,
}

fn history_id(value: &MainRequest) -> Option<&str> { match value { MainRequest::Commit { id } | MainRequest::CommitFiles { id } | MainRequest::CommitFile { id, .. } => Some(id), _ => None } }

impl Navigation {
    pub fn encode(&self) -> String {
        let mut fields = vec!["3".into(), pane(self.focus).into(), pane(self.source).into(), self.file.as_ref().map(path).unwrap_or_default(),
            self.branch.as_deref().map(hex).unwrap_or_default(), self.commit.as_deref().map(hex).unwrap_or_default(),
            self.stash.as_deref().map(hex).unwrap_or_default(), u8::from(self.stash_mode).to_string(), self.cursor.to_string(), self.scroll.to_string(),
            self.request.as_ref().map(request).unwrap_or_default(), hex(&self.queries[0]), hex(&self.queries[1]), hex(&self.queries[2])];
        fields.extend(self.anchors.iter().map(|a| a.as_ref().map(selection).unwrap_or_default()));
        fields.extend(self.anchor_positions.iter().map(|(cursor, scroll)| format!("{cursor},{scroll}")));
        fields.push(self.detail_file.as_ref().map(path).unwrap_or_default());
        fields.push(self.back.len().to_string());
        fields.extend(self.back.iter().map(|frame| format!("{}\t{}\t{}\t{}", frame.cursor, frame.scroll, frame.file.as_ref().map(path).unwrap_or_default(), request(&frame.request))));
        fields.join("\n")
    }

    pub fn decode(value: &str) -> Result<Self, &'static str> {
        if value.len() > 256 * 1024 { return Err("Navigation metadata exceeds its limit; stored record retained"); }
        let f: Vec<_> = value.split('\n').collect();
        if !((f.len() == 11 && f[0] == "1") || (f.len() == 20 && f[0] == "2") || ((22..=24).contains(&f.len()) && f[0] == "3")) { return Err("Unsupported navigation record; stored record retained"); }
        let queries = if f[0] != "1" { [unhex(f[11])?, unhex(f[12])?, unhex(f[13])?] } else { Default::default() };
        if queries.iter().any(|q| q.chars().count() > 256 || q.contains(['\n', '\r'])) { return Err("Invalid saved query; stored record retained"); }
        let optional_text = |value: &str| if value.is_empty() { Ok(None) } else { unhex(value).map(Some) };
        let index = |value: &str| value.parse::<usize>().ok().filter(|n| *n <= 10_000_000).ok_or("Invalid navigation offset; stored record retained");
        let mut anchors = [None, None, None];
        let mut anchor_positions = [(0, 0); 3];
        if f[0] != "1" {
            for i in 0..3 {
                anchors[i] = if f[14+i].is_empty() { None } else { Some(read_selection(f[14+i])?) };
                let (cursor, scroll) = f[17+i].split_once(',').ok_or("Invalid filter anchor offset")?;
                anchor_positions[i] = (index(cursor)?, index(scroll)?);
            }
        }
        let request = if f[10].is_empty() { None } else { Some(read_request(f[10])?) };
        let detail_file = if f[0] == "3" && !f[20].is_empty() { Some(read_path(f[20])?) } else { None };
        let mut back = Vec::new();
        if f[0] == "3" {
            if index(f[21])? != f.len() - 22 { return Err("Invalid history return depth; stored record retained"); }
            for encoded in &f[22..] {
                let fields: Vec<_> = encoded.splitn(4, '\t').collect();
                if fields.len() != 4 { return Err("Invalid history return frame"); }
                back.push(NavigationFrame { request: read_request(fields[3])?, cursor: index(fields[0])?, scroll: index(fields[1])?, file: if fields[2].is_empty() { None } else { Some(read_path(fields[2])?) } });
            }
        } else if let Some(MainRequest::CommitFiles { id } | MainRequest::CommitFile { id, .. }) = &request {
            back.push(NavigationFrame { request: MainRequest::Commit { id: id.clone() }, cursor: 0, scroll: 0, file: None });
            if let Some(MainRequest::CommitFile { file, .. }) = &request { back.push(NavigationFrame { request: MainRequest::CommitFiles { id: id.clone() }, cursor: 2, scroll: 0, file: Some(file.path.clone()) }); }
        }
        for (level, frame) in back.iter().enumerate() {
            if history_id(&frame.request) != request.as_ref().and_then(history_id) || !matches!((level, &frame.request), (0, MainRequest::Commit { .. }) | (1, MainRequest::CommitFiles { .. })) { return Err("History return path does not belong to this commit"); }
        }
        if f[0] == "3" && back.len() != match &request { Some(MainRequest::CommitFiles { .. }) => 1, Some(MainRequest::CommitFile { .. }) => 2, _ => 0 } { return Err("Incomplete history return path"); }
        if request.as_ref().and_then(history_id).is_some() && read_pane(f[2])? != Pane::Commits { return Err("History context belongs to a different source pane"); }
        if detail_file.is_some() && !matches!(request, Some(MainRequest::CommitFiles { .. })) { return Err("Committed file selection has no file list"); }
        Ok(Self { back, detail_file, queries, anchors, anchor_positions, focus: read_pane(f[1])?, source: read_pane(f[2])?, file: if f[3].is_empty() { None } else { Some(read_path(f[3])?) },
            branch: optional_text(f[4])?, commit: optional_text(f[5])?, stash: optional_text(f[6])?,
            stash_mode: match f[7] { "0" => false, "1" => true, _ => return Err("Invalid navigation flag") },
            cursor: index(f[8])?, scroll: index(f[9])?, request })
    }
}

fn selection(value: &NavigationSelection) -> String {
    match value { NavigationSelection::File(p) => format!("file\t{}", path(p)), NavigationSelection::Branch(n) => format!("branch\t{}", hex(n)), NavigationSelection::Commit(id) => format!("commit\t{}", hex(id)) }
}
fn read_selection(value: &str) -> Result<NavigationSelection, &'static str> {
    match value.split_once('\t') { Some(("file", p)) => Ok(NavigationSelection::File(read_path(p)?)), Some(("branch", n)) => Ok(NavigationSelection::Branch(unhex(n)?)), Some(("commit", id)) => Ok(NavigationSelection::Commit(unhex(id)?)), _ => Err("Invalid filter anchor") }
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
