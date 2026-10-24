//! Reviewed literal ignore additions. Git remains the matching/index oracle.
use std::ffi::OsString;
use std::fs::{self, File, Permissions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use minicon_platform::filesystem::{self, FileIdentity, metadata_is_link_like, path_identity};
use srcmgr_domain::NativePath;
use crate::{GitError, GitInstallation, RepositoryIdentity, runner};
use crate::runner::Limits;

const MAX_IGNORE: u64 = 1024 * 1024;

struct Original { identity: FileIdentity, bytes: Vec<u8>, permissions: Permissions }

pub struct IgnoreEdit {
    repository: RepositoryIdentity,
    target: NativePath,
    destination: PathBuf,
    parent_identity: Option<FileIdentity>,
    original: Option<Original>,
    pattern: Vec<u8>,
    tracked: bool,
    duplicate: bool,
    pub lines: Vec<String>,
}

fn refuse(reason: &'static str) -> GitError { GitError::IgnoreEdit(reason) }
fn label(bytes: &[u8]) -> String { NativePath::from_tool_bytes(bytes).map(|p| p.display_label()).unwrap_or_else(|_| format!("bytes: {}", bytes.iter().map(|b|format!("{b:02x}")).collect::<String>())) }

fn literal(path: &NativePath) -> Result<Vec<u8>, GitError> {
    let path = Path::new(path.as_os_str());
    let mut pattern = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else { return Err(refuse("Ignore needs an exact repository-relative path without parent/root components")); };
        #[cfg(unix)]
        let bytes = { use std::os::unix::ffi::OsStrExt; name.as_bytes() };
        #[cfg(not(unix))]
        let bytes = name.to_str().ok_or(GitError::UnrepresentableName)?.as_bytes();
        if bytes.contains(&0) || bytes.contains(&b'\n') || bytes.contains(&b'\r') { return Err(refuse("This filename cannot be represented as one literal ignore rule; no edit was made")); }
        pattern.push(b'/');
        for byte in bytes {
            if matches!(byte,b'\\'|b'*'|b'?'|b'['|b']'|b' '|b'!'|b'#') { pattern.push(b'\\'); }
            pattern.push(*byte);
        }
    }
    if pattern.is_empty() || pattern.len()>65536 { return Err(refuse("Literal ignore rule is empty or exceeds 64 KiB")); }
    Ok(pattern)
}

fn parent_identity(path: &Path) -> Result<Option<FileIdentity>, GitError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !metadata_is_link_like(&m) => path_identity(path).map(Some).map_err(GitError::Io),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound => Ok(None),
        _ => Err(refuse("Ignore destination parent must be a plain directory; links and other file types are refused")),
    }
}

fn original(path: &Path) -> Result<Option<Original>, GitError> {
    let metadata = match fs::symlink_metadata(path) { Ok(m)=>m, Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(None), Err(e)=>return Err(GitError::Io(e)) };
    if !metadata.is_file() || metadata_is_link_like(&metadata) || metadata.len()>MAX_IGNORE { return Err(refuse("Ignore destination must be a plain file of at most 1 MiB; existing data was preserved")); }
    let file = File::open(path).map_err(GitError::Io)?;
    let identity = filesystem::file_identity(&file).map_err(GitError::Io)?;
    if !identity.same_object(path_identity(path).map_err(GitError::Io)?) { return Err(refuse("Ignore destination changed; review again")); }
    let mut bytes=Vec::new();file.take(MAX_IGNORE+1).read_to_end(&mut bytes).map_err(GitError::Io)?;
    if bytes.len() as u64>MAX_IGNORE { return Err(refuse("Ignore destination exceeds 1 MiB; no edit was made")); }
    Ok(Some(Original {identity,bytes,permissions:metadata.permissions()}))
}

fn same_original(before: Option<&Original>, after: Option<&Original>) -> bool {
    match (before,after) { (None,None)=>true, (Some(a),Some(b))=>a.identity.same_object(b.identity)&&a.bytes==b.bytes&&a.permissions==b.permissions, _=>false }
}

fn tracked(git: &GitInstallation, repo: &RepositoryIdentity, target: &NativePath) -> Result<bool,GitError> {
    let args:Vec<OsString>=vec!["--literal-pathspecs".into(),"ls-files".into(),"--error-unmatch".into(),"--".into(),target.as_os_str().to_os_string()];
    match runner::run(git.executable(),Some(&repo.worktree_root),&args,None,Limits::METADATA) { Ok(_)=>Ok(true),Err(GitError::Exit {code:1,..})=>Ok(false),Err(e)=>Err(e) }
}

/// Actual Git result, including a higher-priority negation. Raw -z fields are
/// parsed without inferring state from localized text or colon/tab splitting.
pub fn ignore_effect(git: &GitInstallation, repo: &RepositoryIdentity, target: &NativePath) -> Result<(bool,String),GitError> {
    let args:Vec<OsString>=vec!["check-ignore".into(),"--no-index".into(),"-v".into(),"-z".into(),"--stdin".into()];
    #[cfg(unix)]
    let mut input={use std::os::unix::ffi::OsStrExt;target.as_os_str().as_bytes().to_vec()};
    #[cfg(not(unix))]
    let mut input=target.as_os_str().to_str().ok_or(GitError::UnrepresentableName)?.replace('\\',"/").into_bytes();
    if input.contains(&0){return Err(GitError::UnrepresentableName);}
    input.push(0);
    match runner::run_bytes(git.executable(),Some(&repo.worktree_root),&args,&input,Limits::METADATA) {
        Ok(output)=>{
            let f:Vec<_>=output.stdout.split(|b|*b==0).collect();
            if f.len()!=5 || !f[4].is_empty() {return Err(GitError::MalformedOutput {record:0});}
            let ignored=!f[2].starts_with(b"!");
            Ok((ignored,format!("{}:{}: {}{}",label(f[0]),label(f[1]),label(f[2]),if ignored {""} else {" (re-includes this path)"})))
        }
        Err(GitError::Exit {code:1,..})=>Ok((false,"No matching ignore rule".into())),Err(e)=>Err(e),
    }
}

fn check_target(repo:&RepositoryIdentity,target:&NativePath,was_tracked:bool,git:&GitInstallation)->Result<(),GitError> {
    if tracked(git,repo,target)?!=was_tracked {return Err(refuse("Tracking state changed; review Ignore again"));}
    let parts:Vec<_>=Path::new(target.as_os_str()).components().collect();
    let mut path=repo.worktree_root.clone();
    for (index,part) in parts.iter().enumerate() {
        path.push(part.as_os_str());
        match fs::symlink_metadata(&path) {
            Ok(m) if index+1<parts.len() && (!m.is_dir()||metadata_is_link_like(&m))=>return Err(refuse("Ignore path traverses a link or unsupported directory")),
            Ok(m) if index+1==parts.len() && m.is_dir()=>return Err(refuse("Select one file; directory-wide ignore is a separate scope")),
            Ok(_)=>{},
            Err(e) if e.kind()==std::io::ErrorKind::NotFound && was_tracked=>{},
            Err(_)=>return Err(refuse("Selected file disappeared; refresh and select again")),
        }
    }
    Ok(())
}

impl IgnoreEdit {
    pub fn prepare(git:&GitInstallation,repo:&RepositoryIdentity,target:&NativePath,local:bool)->Result<Self,GitError> {
        git.require_supported()?;
        let pattern=literal(target)?;
        let tracked=tracked(git,repo,target)?;check_target(repo,target,tracked,git)?;
        let destination=if local {repo.common_dir.join("info/exclude")} else {repo.worktree_root.join(".gitignore")};
        let parent=destination.parent().unwrap();
        let parent_identity=parent_identity(parent)?;
        let original=original(&destination)?;
        let duplicate=original.as_ref().is_some_and(|o|o.bytes.split(|b|*b==b'\n').any(|line|line.strip_suffix(b"\r").unwrap_or(line)==pattern));
        let (_,effect)=ignore_effect(git,repo,target)?;
        let lines=vec![format!("Exact selected path: {}",target.display_label()),format!("Destination: {}",NativePath::new(destination.as_os_str()).display_label()),format!("Literal rule: {}",label(&pattern)),format!("Current rule (as if untracked): {effect}"),if tracked {"This file is tracked. Ignore does not stop tracking it or change its index entry.".into()} else {"Only this exact untracked path is targeted; no extension wildcard is added.".into()},if local {"Repository-local excludes are not committed. Higher-priority project rules can override them.".into()} else {"Project .gitignore is ordinary versionable content. It is not staged or committed automatically.".into()},if duplicate {"This exact rule already exists; no duplicate line will be written.".into()} else {"Append one rule, preserving all existing bytes/comments. Nested rules may override it; report actual Git matching after save.".into()},if parent_identity.is_none() {"The missing info directory will be created only after approval.".into()} else {"Existing directories and rules remain in place.".into()}];
        Ok(Self {repository:repo.clone(),target:target.clone(),destination,parent_identity,original,pattern,tracked,duplicate,lines})
    }

    fn check(&self,git:&GitInstallation,repo:&RepositoryIdentity)->Result<(),GitError> {
        if repo!=&self.repository {return Err(refuse("Ignore belongs to another repository"));}
        check_target(repo,&self.target,self.tracked,git)?;
        let parent=parent_identity(self.destination.parent().unwrap())?;
        if !match (self.parent_identity,parent) {(None,None)=>true,(Some(a),Some(b))=>a.same_object(b),_=>false} || !same_original(self.original.as_ref(),original(&self.destination)?.as_ref()) {return Err(refuse("Ignore file or parent changed; review again"));}
        Ok(())
    }

    /// No fallible operation after publication can turn a known write into an
    /// apparent refusal. Cleanup/durability/matching limitations become notes.
    pub fn publish(self,git:&GitInstallation,repo:&RepositoryIdentity)->Result<String,GitError> {
        self.check(git,repo)?;
        if self.duplicate {return Ok("Ignore rule already exists; no file was changed. Tracking and index are unchanged.".into());}
        let parent=self.destination.parent().unwrap();
        let mut created=CreatedDirectory {path:parent.to_owned(),identity:None};
        if self.parent_identity.is_none() {
            fs::create_dir(parent).map_err(GitError::Io)?;
            created.identity=Some(path_identity(parent).map_err(GitError::Io)?);
        }
        let expected_parent=parent_identity(parent)?.ok_or_else(||refuse("Ignore parent disappeared"))?;
        let lock_path=self.destination.with_file_name(format!("{}.lock",self.destination.file_name().unwrap().to_string_lossy()));
        let mut file=filesystem::private_create_new_options().open(&lock_path).map_err(GitError::Io)?;
        let mut lock=OwnedLock {path:lock_path,identity:filesystem::file_identity(&file).map_err(GitError::Io)?,armed:true};
        let old=self.original.as_ref().map_or(&b""[..],|o|o.bytes.as_slice());
        let newline=if old.windows(2).any(|w|w==b"\r\n") {&b"\r\n"[..]} else {&b"\n"[..]};
        let mut bytes=old.to_vec();
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {if bytes.ends_with(b"\r") {bytes.push(b'\n');}else {bytes.extend_from_slice(newline);}}
        bytes.extend_from_slice(&self.pattern);bytes.extend_from_slice(newline);
        if bytes.len() as u64>MAX_IGNORE {return Err(refuse("Updated ignore file exceeds 1 MiB; no edit was published"));}
        file.write_all(&bytes).map_err(GitError::Io)?;
        if let Some(original)=&self.original {file.set_permissions(original.permissions.clone()).map_err(GitError::Io)?;}
        file.sync_all().map_err(GitError::Io)?;drop(file);
        check_target(repo,&self.target,self.tracked,git)?;
        if !expected_parent.same_object(path_identity(parent).map_err(GitError::Io)?) || !same_original(self.original.as_ref(),original(&self.destination)?.as_ref()) {return Err(refuse("Ignore changed before publication; no edit was published"));}
        let staged=original(&lock.path)?.ok_or_else(||refuse("Owned ignore lock disappeared"))?;
        if !staged.identity.same_object(lock.identity) || staged.bytes!=bytes {return Err(refuse("Owned ignore lock changed; no edit was published"));}
        if self.original.is_some() {filesystem::replace_file(&lock.path,&self.destination).map_err(GitError::Io)?;lock.armed=false;}
        else {fs::hard_link(&lock.path,&self.destination).map_err(GitError::Io)?;}
        created.identity=None;
        let cleanup_warning=if lock.armed {match path_identity(&lock.path) {Ok(id) if id.same_object(lock.identity)=>fs::remove_file(&lock.path).is_err(),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>false,_=>true}} else {false};lock.armed=false;
        let durability_warning=filesystem::sync_parent(parent).is_err();
        let observed=if self.tracked {"File remains tracked; ignore does not change its index entry.".into()} else {match ignore_effect(git,repo,&self.target) {Ok((true,rule))=>format!("Git now ignores the selected path: {rule}"),Ok((false,rule))=>format!("Rule saved, but Git still includes the selected path: {rule}"),Err(_)=>"Rule saved; current ignore matching could not be verified. Refresh and inspect before further actions.".into()}};
        Ok(format!("Ignore rule saved. {observed}{}{} Only the reviewed ignore destination was edited; HEAD and index were kept.",if durability_warning {" Parent-directory durability could not be confirmed."} else {""},if cleanup_warning {" An ignore lock remains; inspect it before another edit."} else {""}))
    }
}

struct OwnedLock {path:PathBuf,identity:FileIdentity,armed:bool}
impl Drop for OwnedLock {fn drop(&mut self){if self.armed && path_identity(&self.path).is_ok_and(|id|id.same_object(self.identity)){let _=fs::remove_file(&self.path);}}}
struct CreatedDirectory {path:PathBuf,identity:Option<FileIdentity>}
impl Drop for CreatedDirectory {fn drop(&mut self){if let Some(expected)=self.identity && path_identity(&self.path).is_ok_and(|id|id.same_object(expected)){let _=fs::remove_dir(&self.path);}}}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::process::{Command,Output};
    use std::sync::atomic::{AtomicU64,Ordering};
    struct Fixture {root:PathBuf,repo:RepositoryIdentity,git:GitInstallation}
    impl Fixture {
        fn command(&self,args:&[&OsStr])->Output {
            let mut cmd=Command::new(self.git.executable());cmd.current_dir(&self.repo.worktree_root).args(args);
            for (key,_) in std::env::vars_os(){if key.to_string_lossy().starts_with("GIT_"){cmd.env_remove(key);}}
            cmd.env("GIT_CONFIG_GLOBAL",self.root.join("global")).env("GIT_CONFIG_NOSYSTEM","1").output().unwrap()
        }
        fn git(&self,args:&[&str])->Output {self.command(&args.iter().map(OsStr::new).collect::<Vec<_>>())}
        fn new()->Self {
            static NEXT:AtomicU64=AtomicU64::new(0);
            let root=std::env::temp_dir().join(format!("srcmgr-ignore-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
            fs::create_dir(&root).unwrap();fs::write(root.join("global"),b"").unwrap();fs::create_dir(root.join("empty-template")).unwrap();
            let dir=root.join("repo");fs::create_dir(&dir).unwrap();
            let exe=std::env::var_os("SRCMGR_TEST_GIT").unwrap_or_else(||"git".into());
            let git=GitInstallation::probe(Path::new(&exe)).unwrap();
            let output=Command::new(git.executable()).current_dir(&dir).args(["init","-q","--initial-branch=main","--template"]).arg(root.join("empty-template")).env("GIT_CONFIG_GLOBAL",root.join("global")).env("GIT_CONFIG_NOSYSTEM","1").output().unwrap();assert!(output.status.success());
            let repo=crate::locate(&git,&dir).unwrap();let fixture=Self {root,repo,git};
            fs::write(fixture.repo.worktree_root.join("a.txt"),b"tracked\n").unwrap();
            fs::write(fixture.repo.worktree_root.join(".gitignore"),b"# original\r\n# keep\r\n").unwrap();
            for args in [&["config","user.name","Fixture"][..],&["config","user.email","fixture@example.invalid"][..],&["add","a.txt",".gitignore"][..],&["commit","-qm","initial"][..]] {assert!(fixture.git(args).status.success());}
            fixture
        }
        fn ignored(&self,path:&OsStr)->bool {self.command(&[OsStr::new("check-ignore"),OsStr::new("-q"),OsStr::new("--"),path]).status.success()}
    }
    impl Drop for Fixture {fn drop(&mut self){let _=fs::remove_dir_all(&self.root);}}

    #[test]
    fn ignore_literal_scopes_preserve_index_comments_and_refuse_stale_or_locked_files() {
        let f=Fixture::new();let root=&f.repo.worktree_root;
        let selected=NativePath::new("cache[1] 中文.tmp");let sibling=OsStr::new("cache1 中文.tmp");
        for name in [selected.as_os_str(),sibling,OsStr::new("local.cache"),OsStr::new("override.cache")] {fs::write(root.join(name),b"untouched").unwrap();}
        fs::create_dir(root.join("nested")).unwrap();fs::write(root.join("nested").join(selected.as_os_str()),b"nested").unwrap();
        let before_tree=f.git(&["write-tree"]).stdout;let before_head=f.git(&["rev-parse","HEAD"]).stdout;
        let project=root.join(".gitignore");let original=fs::read(&project).unwrap();
        let edit=IgnoreEdit::prepare(&f.git,&f.repo,&selected,false).unwrap();assert_eq!(fs::read(&project).unwrap(),original);
        edit.publish(&f.git,&f.repo).unwrap();
        assert!(!f.ignored(sibling),"a sibling matching an unescaped glob must not become ignored");
        assert!(f.ignored(selected.as_os_str()));assert!(!f.ignored(OsStr::new("nested/cache[1] 中文.tmp")));
        assert!(fs::read(&project).unwrap().starts_with(&original));assert_eq!(fs::read(root.join(selected.as_os_str())).unwrap(),b"untouched");
        let once=fs::read(&project).unwrap();IgnoreEdit::prepare(&f.git,&f.repo,&selected,false).unwrap().publish(&f.git,&f.repo).unwrap();assert_eq!(fs::read(&project).unwrap(),once);
        let stale=IgnoreEdit::prepare(&f.git,&f.repo,&NativePath::new(sibling),false).unwrap();
        let mut external=once.clone();external.extend_from_slice(b"# concurrent comment\r\n");fs::write(&project,&external).unwrap();
        let stale_result=stale.publish(&f.git,&f.repo);assert_eq!(fs::read(&project).unwrap(),external,"a stale preview must preserve the other writer's exact rules");assert!(stale_result.is_err());
        let edit=IgnoreEdit::prepare(&f.git,&f.repo,&NativePath::new(sibling),false).unwrap();let lock=root.join(".gitignore.lock");fs::write(&lock,b"other writer").unwrap();
        assert!(edit.publish(&f.git,&f.repo).is_err());assert_eq!(fs::read(&lock).unwrap(),b"other writer");assert_eq!(fs::read(&project).unwrap(),external);fs::remove_file(lock).unwrap();
        IgnoreEdit::prepare(&f.git,&f.repo,&NativePath::new("local.cache"),true).unwrap().publish(&f.git,&f.repo).unwrap();assert!(f.ignored(OsStr::new("local.cache")));assert_eq!(fs::read(&project).unwrap(),external);
        let tracked=IgnoreEdit::prepare(&f.git,&f.repo,&NativePath::new("a.txt"),true).unwrap().publish(&f.git,&f.repo).unwrap();assert!(tracked.contains("remains tracked"));assert!(f.git(&["ls-files","--error-unmatch","a.txt"]).status.success());
        let mut with_negation=external.clone();with_negation.extend_from_slice(b"!override.cache\r\n");fs::write(&project,&with_negation).unwrap();
        let result=IgnoreEdit::prepare(&f.git,&f.repo,&NativePath::new("override.cache"),true).unwrap().publish(&f.git,&f.repo).unwrap();assert!(result.contains("still includes"));assert!(!f.ignored(OsStr::new("override.cache")));
        assert_eq!(f.git(&["write-tree"]).stdout,before_tree);assert_eq!(f.git(&["rev-parse","HEAD"]).stdout,before_head);assert_eq!(fs::read(root.join("a.txt")).unwrap(),b"tracked\n");
    }

    #[cfg(unix)]
    #[test]
    fn ignore_raw_unix_names_and_link_refusals_keep_exact_bytes() {
        use std::os::unix::{ffi::OsStringExt,fs::symlink};
        let f=Fixture::new();let root=&f.repo.worktree_root;
        let raw=NativePath::new(OsString::from_vec(vec![b'r',0xff,b'[',b'1',b']']));fs::write(root.join(raw.as_os_str()),b"raw").unwrap();
        IgnoreEdit::prepare(&f.git,&f.repo,&raw,false).unwrap().publish(&f.git,&f.repo).unwrap();assert!(f.ignored(raw.as_os_str()));
        let original=fs::read(root.join(".gitignore")).unwrap();
        fs::write(root.join("line\nbreak"),b"keep").unwrap();assert!(IgnoreEdit::prepare(&f.git,&f.repo,&NativePath::new("line\nbreak"),false).is_err());assert_eq!(fs::read(root.join(".gitignore")).unwrap(),original);
        let outside=f.root.join("outside");fs::write(&outside,b"outside bytes").unwrap();fs::remove_file(root.join(".gitignore")).unwrap();symlink(&outside,root.join(".gitignore")).unwrap();
        assert!(IgnoreEdit::prepare(&f.git,&f.repo,&raw,false).is_err());assert_eq!(fs::read(outside).unwrap(),b"outside bytes");
    }
}
