//! Product-owned message keys and their two delivered locales.
//!
//! Keys are semantic Rust symbols, never persisted as enum ordinals, so a
//! message can be reworded without touching stored state. Each key carries both
//! strings in one place: the macro cannot compile a key that lacks a translation,
//! which is the whole coverage guarantee — there is no second table to drift.
//!
//! What a locale may change is display text and only display text. Paths, object
//! ids, refspecs, refs and Git's own error output stay literal in both locales,
//! because translating any of them would change what the product reports about
//! the repository rather than how it words a sentence.
//!
//! Static lookup keeps this free of a service, a parser, an allocation or a
//! mutable global: the locale is a value the caller passes down.

/// The delivered interface languages. `System` resolves once at startup and is
/// then replaced by the concrete language, so a rendered frame never depends on
/// an environment lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Locale {
    En,
    ZhCn,
}

impl Locale {
    /// The value persisted in settings and shown in the selector.
    pub const fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCn => "zh-CN",
        }
    }

    /// Parses a persisted tag. An unknown value falls back to English rather
    /// than failing, so a downgrade cannot lock the interface.
    pub const fn from_tag(tag: &str) -> Self {
        match tag.as_bytes() {
            b"zh-CN" | b"zh_CN" | b"zh" => Self::ZhCn,
            _ => Self::En,
        }
    }

    /// Resolves `System` against the two languages this build ships. Anything
    /// whose language is not Chinese stays English.
    pub fn resolve(requested: Option<&str>, chinese_environment: bool) -> Self {
        match requested {
            // "system" and an unset value are the same instruction: follow the
            // environment once, here, so a rendered frame never re-reads it.
            None | Some("system") => {
                if chinese_environment {
                    Self::ZhCn
                } else {
                    Self::En
                }
            }
            Some(tag) => Self::from_tag(tag),
        }
    }
}

macro_rules! catalog {
    ($($key:ident => $en:literal, $zh:literal;)*) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum Key { $($key),* }

        pub const fn english(key: Key) -> &'static str {
            match key { $(Key::$key => $en),* }
        }

        pub const fn chinese(key: Key) -> &'static str {
            match key { $(Key::$key => $zh),* }
        }

        impl Key {
            /// Every key, in declaration order. The coverage test walks this, so
            /// a key added without a counterpart cannot hide. Compiled only for
            /// tests because nothing else enumerates the catalogue.
            #[cfg(test)]
            pub(crate) const ALL: &'static [Key] = &[$(Key::$key),*];
        }
    };
}

pub const fn text(key: Key, locale: Locale) -> &'static str {
    match locale {
        Locale::En => english(key),
        Locale::ZhCn => chinese(key),
    }
}

catalog! {
    ActionOpen => "Open", "打开";
    ActionBack => "Back", "返回";
    ActionCancel => "Cancel", "取消";
    ActionClose => "Close", "关闭";
    EntryCreate => "Create", "创建";
    EntryGitSettings => "Git settings", "Git 设置";
    EntryIdentitySettings => "Identity settings", "身份设置";
    EntryOpenTitle => "Open repository", "打开仓库";
    EntryRepositoryPath => "Repository path", "仓库路径";
    EntryGitExecutable => "Git executable (blank = PATH)", "Git 可执行文件（留空则用 PATH）";
    EntryApplicationGitSettings => "Application Git settings", "应用级 Git 设置";
    EntryTestSave => "Test and save", "测试并保存";
    EntrySelectionDiagnostics => "Selection and diagnostics", "选择与诊断";
    EntryWorking => "Working...", "处理中…";
    EntryFinishIme => "Finish IME", "结束输入法";
    CreateTitle => "Create local Git repository", "创建本地 Git 仓库";
    CreateDirectory => "Directory (parent must exist)", "目录（上级目录须已存在）";
    CreateBranch => "Initial branch", "初始分支";
    CreateReviewTitle => "Review repository creation", "复核仓库创建";
    CreateReview => "Review creation", "复核创建";
    CreateApprove => "Create repository", "创建仓库";
    CreateBefore => "Before creating", "创建前";
    IdentityTitle => "Repository identity (local scope)", "仓库身份（本地范围）";
    IdentityName => "Name", "姓名";
    IdentityEmail => "Email", "邮箱";
    IdentitySources => "Effective identity and config sources", "生效身份与配置来源";
    IdentitySourcesScroll => "Effective identity and config sources (scroll)", "生效身份与配置来源（可滚动）";
    IdentityReload => "Reload preview", "重新载入预览";
    IdentitySave => "Save local", "保存到本地";
    TrustReview => "Repository trust review", "仓库信任复核";
    TrustReviewTitle => "Review repository trust", "复核仓库信任";
    TrustApprove => "Trust and open", "信任并打开";
    CommitPrepare => "Prepare commit", "准备提交";
    CommitMessage => "Commit message", "提交说明";
    CommitApprove => "Commit", "提交";
    CommitDisabled => "Commit disabled", "提交已禁用";
    CommitRefresh => "Refresh review", "刷新复核";
    CommitClose => "Save and close", "保存并关闭";
    CommitIndex => "Complete staged index", "完整暂存区索引";
    CommitDiff => "Staged Diff", "暂存差异";
    ChangesStage => "Stage", "暂存";
    ChangesUnstage => "Unstage", "取消暂存";
    ChangesDiscard => "Discard", "丢弃";
    ChangesIgnore => "Ignore", "忽略";
    ChangesProjectIgnore => "Project .gitignore", "项目 .gitignore";
    ChangesLocalExclude => "Local exclude", "本地排除";
    ChangesSaveRule => "Save rule", "保存规则";
    ChangesConfirm => "Confirm reviewed action", "确认已复核的操作";
    ChangesReview => "Action review", "操作复核";
    ChangesVisibleDiff => "Selected Diff visible content", "所选差异的可见内容";
    CurrentActionStatus => "Current action status", "当前操作状态";
    RecentOpen => "Open selected", "打开所选";
    RecentLocate => "Locate recent", "定位最近项";
    RecentTitle => "Recent repositories", "最近仓库";
    RecentEmpty => "No recent repositories", "没有最近仓库";
    WelcomeTitle => "Welcome", "欢迎";
    BranchNewTitle => "New branch name", "新分支名称";
    FilterClear => "Clear", "清除";
    LocateReview => "Review location", "复核定位";
    LocateSave => "Save location", "保存定位";
    RecoveryArchiveOpen => "Archive view and open", "归档视图并打开";
    ToolsSave => "Save", "保存";
    ToolsReview => "Review launch", "复核启动";
    PanelReview => "Review", "复核";
    NoteTabsFull => "16 repositories are open; close a tab before opening another", "已打开 16 个仓库；请先关闭一个标签页再打开新的";
    NoteClearFilterScope => "Clear Files filter to review the full operation scope", "清除文件过滤器以复核完整操作范围";
    NoteCommitRunning => "Commit is running; wait for its observed result before closing", "提交正在进行；请等待观察到结果后再关闭";
    NoteCreateCancelled => "Creation cancelled; no directory or metadata was created", "已取消创建；未创建任何目录或元数据";
    NoteFilterLimit => "Filter limit is 256 Unicode characters on one line; input was not inserted", "过滤条件上限为单行 256 个 Unicode 字符；未插入输入";
    NoteImeSwitchRepo => "Finish IME composition before changing repositories", "切换仓库前请先结束输入法组合";
    NoteImeClearFilter => "Finish IME composition before clearing the filter", "清除过滤条件前请先结束输入法组合";
    NoteImeLeaveFilter => "Finish IME composition before leaving the filter", "离开过滤条件前请先结束输入法组合";
    NoteImeEditorCommand => "Finish IME composition before using editor commands", "使用编辑命令前请先结束输入法组合";
    NoteDialogBeforeSwitch => "Finish or cancel this dialog before changing repositories", "切换仓库前请先完成或取消此对话框";
    NoteGitPathLimit => "Git path limit is 32 KiB UTF-8; input was not appended", "Git 路径上限为 32 KiB UTF-8；未追加输入";
    NoteIdentityLimit => "Identity field limit is 1024 UTF-8 bytes; input was not appended", "身份字段上限为 1024 UTF-8 字节；未追加输入";
    NoteLoadingIndex => "Loading complete staged-index review...", "正在载入完整暂存区复核…";
    NoteMessageLimit => "Message limit is 64 KiB UTF-8; input was not inserted", "提交说明上限为 64 KiB UTF-8；未插入输入";
    NoteNoRemote => "No remote is configured; add one with Git or clone a repository", "尚未配置远端；请用 Git 添加，或克隆一个仓库";
    NoteOpenBeforePull => "Open a repository before pulling", "请先打开仓库再拉取";
    NoteOpenCancelled => "Opening cancelled; saved view was retained", "已取消打开；已保留保存的视图";
    NotePreviousFileMissing => "Previous committed file is unavailable in this comparison; current list is displayed", "此比较中上一个已提交文件不可用；已显示当前列表";
    NoteRepositoryCreated => "Repository created. Review trust before opening; existing files remain unstaged.", "仓库已创建。打开前请复核信任；已有文件仍未暂存。";
    NoteSavedDetailStale => "Saved detail is outside the current loaded/filter state or changed side; current selection was refreshed", "保存的详情超出当前加载或过滤状态，或已换侧；已刷新当前选择";
    NoteSavedPathNotUtf8 => "Saved native Git path cannot be edited as UTF-8; it was preserved. Enter a replacement explicitly or cancel.", "保存的原生 Git 路径无法以 UTF-8 编辑；已保留。请显式输入替代值或取消。";
    NoteBackendUnavailable => "This backend is not available; the record was preserved", "此后端不可用；记录已保留";
    NoteBackendRecentPreserved => "This backend is unavailable; its recent record was preserved", "此后端不可用；其最近记录已保留";
    NoteForeignPlatform => "This location belongs to another platform; select a local path", "此位置属于其他平台；请选择本地路径";
    NotePathAdapterUnavailable => "This native path is unavailable with the current Git adapter; no lossy conversion was used", "当前 Git 适配器不支持此原生路径；未使用有损转换";
    NoteTrustCancelled => "Trust cancelled; repository opening was not requested", "已取消信任；未请求打开仓库";
    NoteWaitSaveBeforeSwitch => "Wait for the current save or commit before switching repositories", "切换仓库前请等待当前保存或提交完成";
    ViewWelcome => "Welcome to MiniCon SrcMgr\\nOpen a local Git repository with Ctrl+O or Open.\\nThere are no recent repositories yet.\\nAn installed Git is needed only when opening a repository.\\nNo account or network is required for local work.", "欢迎使用 MiniCon SrcMgr\\n用 Ctrl+O 或“打开”打开本地 Git 仓库。\\n目前还没有最近仓库。\\n只有在打开仓库时才需要已安装的 Git。\\n本地工作不需要账号或网络。";
    ViewOpenScreen => "Open a repository by path, or select a recent location.\\nRecent locations are not trusted automatically.\\nNo repository commands run while browsing this screen.\\n\\nCtrl+O: Open path\\nEnter: Open selected recent location\\nL: Locate selected recent location\\nd: Remove selected recent record only\\nr: Reload recent records", "按路径打开仓库，或选择一个最近位置。\\n最近位置不会自动获得信任。\\n停留在此界面时不会执行任何仓库命令。\\n\\nCtrl+O：打开路径\\nEnter：打开选中的最近位置\\nL：重新定位选中的最近位置\\nd：仅移除选中的最近记录\\nr：重新载入最近记录";
    ViewRecentDetail => "Recent location ({backend})\\n{label}\\n\\nNot checked or trusted by listing it.\\nOpen: inspect this location through the ordinary trust gate.\\nLocate: replace this bookmark's path after review.\\nRemove: remove this navigation record only; keep files/drafts/trust.", "最近位置（{backend}）\\n{label}\\n\\n列出并不代表已检查或已信任。\\nOpen：通过常规信任门检查此位置。\\nLocate：复核后替换此书签路径。\\nRemove：仅移除此导航记录；保留文件、草稿与信任。";
    StatusDiverged => " | diverged: local {local} | fetched upstream {upstream}, as of the last fetch {stamp}", " | 已分叉：本地 {local} | 已抓取的上游 {upstream}，截至上次抓取 {stamp}";
    StatusDivergedNoTime => " | diverged: local {local} | fetched upstream {upstream} (fetch time unavailable)", " | 已分叉：本地 {local} | 已抓取的上游 {upstream}（抓取时间不可用）";
    StatusInProgress => " | {operation} in progress", " | {operation} 正在进行";
    StatusInProgressUnmerged => " | {operation} in progress, {count} unmerged", " | {operation} 正在进行，{count} 个未合并";
    StatusUnmerged => " | {count} unmerged", " | {count} 个未合并";
    NoteRefreshCancelled => "Repository state refreshed; confirmation cancelled. Preview the action again.", "仓库状态已刷新；确认已取消。请重新预览该操作。";
    NoteDraftSaved => "Draft saved", "草稿已保存";
    NoteNothingToStage => "nothing to stage or unstage", "没有可暂存或取消暂存的内容";
    NoteSpaceFileDiff => "Space acts on a file diff", "空格键作用于文件差异";
    NoteNoHunks => "this diff has no hunks", "此差异没有可操作的区块";
    NoteSelectChangedLine => "select a changed line in the diff", "请在差异中选择一行已更改内容";
    NoteAActsOnFileDiff => "a acts on a file diff", "a 键作用于文件差异";
    NoteDiscardFileDiff => "discard works on a file diff", "丢弃作用于文件差异";
    NoteDiscardUnstagedOnly => "discard works on unstaged changes; this view shows staged ones", "丢弃只作用于未暂存更改；此视图显示的是已暂存内容";
    NoteDiscardUntracked => "an untracked file is discarded from the Files panel", "未跟踪文件请在文件面板中丢弃";
    NoteNoHunkToDiscard => "this change has no hunk to discard", "此更改没有可丢弃的区块";
    NoteNoTrackedToStash => "no tracked changes to stash", "没有已跟踪的更改可贮藏";
    NoteFieldLimit => "Field limit is {limit} UTF-8 bytes; input was not appended", "字段上限为 {limit} UTF-8 字节；未追加输入";
    NoteRemoteApplied => "Applied.{detail} The repository configuration changed, so its trust fingerprint changed with it. Reopen the repository from Recent to review the new fingerprint; until then no Git action will run.", "已应用。{detail} 仓库配置已变更，其信任指纹随之改变。请从最近列表重新打开仓库以复核新指纹；在此之前不会执行任何 Git 操作。";
    NoteCloned => "Cloned into {destination}. The working tree is not checked out: open the repository and approve trust to review the filter and check it out.{detail}", "已克隆到 {destination}。工作树尚未检出：请打开该仓库并通过信任复核来检查过滤器并检出。{detail}";
    NoteCheckoutDone => "Working tree checked out from HEAD; the repository now has its files.", "工作树已从 HEAD 检出；仓库现在已有文件。";
    RefuseOpInProgress => "Refused: {operation} is in progress. Resolve it in a terminal, then refresh. Press t in the window to open a configured terminal at this repository.", "已拒绝：{operation} 正在进行。请在终端中解决后刷新。在窗口中按 t 可打开已配置的终端。";
    RefuseOpUnmerged => "Refused: {count} path(s) are unmerged. Resolve them in a terminal, then refresh. Press t in the window to open a configured terminal at this repository.", "已拒绝：{count} 个路径未合并。请在终端中解决后刷新。在窗口中按 t 可打开已配置的终端。";
    RefuseOpBoth => "Refused: {operation} is in progress and {count} path(s) are unmerged. Resolve them in a terminal, then refresh. Press t in the window to open a configured terminal at this repository.", "已拒绝：{operation} 正在进行，且有 {count} 个路径未合并。请在终端中解决后刷新。在窗口中按 t 可打开已配置的终端。";
    RefuseOpUnreadable => "Refused: the repository's operation state cannot be read ({detail}); inspect it before any write.", "已拒绝：无法读取仓库的操作状态（{detail}）；请先检查再进行任何写入。";
    RefusePath => "Repository: {path}", "仓库：{path}";
    RefuseDiverged => "Refused: the local branch and its fetched upstream have both advanced.\\nLocal: {local}\\nFetched upstream: {upstream}\\nPull never merges, rebases or stashes on its own; resolve the divergence in a terminal, then refresh.", "已拒绝：本地分支与已抓取的上游都已前进。\\n本地：{local}\\n已抓取的上游：{upstream}\\n拉取不会自行合并、变基或贮藏；请在终端中解决分歧后刷新。";
    PaneStatusTitle => "[1] Status", "[1] 状态";
    PaneFilesLabel => "Files", "文件";
    PaneBranchesLabel => "Branches", "分支";
    PaneListTitle => "[{number}] {name} ({count})", "[{number}] {name}（{count} 项）";
    PaneCommitsTitle => "[4] Loaded commits {count} max{limit}", "[4] 已载入提交 {count}，上限 {limit}";
    PaneStashesTitle => "[5] Stashes ({count}; max 200)", "[5] 贮藏（{count}；上限 200）";
    PaneStashesEmpty => "No stashes", "没有贮藏";
    CloneFailedTitle => "Clone failed", "克隆失败";
    CloneTargetLine => "Target: {destination}", "目标：{destination}";
    NoteCheckoutExpired => "The checkout review expired or was already used; review it again.", "检出复核已过期或已被使用；请重新复核。";
    NoteRemoteReviewExpired => "The remote review expired or was already used; review the change again.", "远端复核已过期或已被使用；请重新复核该变更。";
    RefuseIgnoreNeedsReview => "Ignore needs a fresh reviewed confirmation", "忽略操作需要一次新的复核确认";
    RefuseCommittedFileScope => "committed file does not belong to the selected comparison", "该已提交文件不属于所选比较";
    NoteCommitNeedsReconcile => "Previous commit attempt needs reconciliation. Refresh repository state before opening another commit review; the draft is preserved.", "上一次提交尝试需要先核对。请刷新仓库状态后再打开新的提交复核；草稿已保留。";
    NoteCommitReviewUnavailable => "Commit review is unavailable; refresh it", "提交复核不可用；请刷新";
    NoteCommitReviewSelectAgain => "Commit review changed; select again", "提交复核已变更；请重新选择";
    NoteCommitReviewRefresh => "Commit review changed; refresh before submitting", "提交复核已变更；提交前请刷新";
    NoteCommitMessageEmpty => "the commit message is empty", "提交说明为空";
    NoteLoadReviewFirst => "Load the complete staged-index review before committing", "提交前请先载入完整暂存区复核";
    ResultRefusedBefore => "Action refused before execution; state refreshed: {error}", "操作在执行前被拒绝；状态已刷新：{error}";
    ResultActionFailed => "Action failed; state refreshed: {error}", "操作失败；状态已刷新：{error}";
    ResultRecentNotUpdated => "Recent list not updated: {error}; repository data was not changed", "最近列表未更新：{error}；仓库数据未改动";
    ResultFetchCurrent => "{remote}: up to date", "{remote}：已是最新";
    ResultFetchUpdated => "{remote}: {count} refs updated", "{remote}：已更新 {count} 个引用";
    ResultCommitObserved => "Observed commit OID: {id}{warning}", "已观察到的提交 OID：{id}{warning}";
    HintCheckoutReview => "y Check out   n/Esc Cancel   Page keys Scroll   Enter does not approve", "y 检出   n/Esc 取消   Page 滚动   Enter 不会批准";
    RefuseActiveRepoChanged => "Active repository changed; identity was not saved", "活动仓库已变更；身份未保存";
    NoteCloneApprovalExpired => "The clone review expired or was already used; review the clone again.", "克隆复核已过期或已被使用；请重新复核。";
    RefuseOpenBeforeWork => "Open a repository before requesting repository work", "请先打开仓库再请求仓库操作";
    ResultCloneNotStarted => "Clone not started: {text}", "克隆未开始：{text}";
    ResultLanguageNotSaved => "Language not saved: {text}", "语言未保存：{text}";
    RefuseActiveRepoRequired => "Active repository context is required before entry", "进入前需要活动仓库上下文";
    RefuseBranchNameInvalid => "Choose a valid initial branch name (up to 255 UTF-8 bytes)", "请选择有效的初始分支名（最多 255 个 UTF-8 字节）";
    RefuseGitOutsideRepos => "Choose an installed Git outside the selected repositories", "请选择不在所选仓库内的已安装 Git";
    RefuseCreateReviewExpired => "Creation review expired or changed; review again", "创建复核已过期或已变更；请重新复核";
    RefuseAbsolutePath => "Enter an absolute local repository path", "请输入本地仓库的绝对路径";
    RefuseIdentityPreviewExpired => "Identity preview expired or changed; reload before saving", "身份预览已过期或已变更；保存前请重新载入";
    RefuseLocationReviewExpired => "Location review expired or changed; review again", "定位复核已过期或已变更；请重新复核";
    RefuseNavigationOtherRepo => "Navigation belongs to another repository; target unchanged", "该导航属于其他仓库；目标未变更";
    RefuseNavigationRecoveryExpired => "Navigation recovery changed or expired; review again", "导航恢复已变更或过期；请重新复核";
    RefuseRecentEntryChanged => "Recent entry changed; reload the list before locating again", "最近记录已变更；重新定位前请先刷新列表";
    RefuseRecentListUnavailable => "Recent list unavailable", "最近列表不可用";
    RefuseMetadataChangedLocate => "Repository metadata changed; review the location again", "仓库元数据已变更；请重新复核该定位";
    RefuseSavedViewChanged => "Saved view changed; retry opening to load the current record", "保存的视图已变更；请重试打开以载入当前记录";
    RefuseSelectGitFile => "Select a Git executable file", "请选择 Git 可执行文件";
    RefuseGitInsideTarget => "Select an installed Git outside the initialization target", "请选择不在初始化目标内的已安装 Git";
    RefuseBackendRecentEntry => "This backend is unavailable; the recent entry was preserved", "此后端不可用；最近记录已保留";
    RefuseMetadataChangedTrust => "repository metadata changed after review; open and review again", "仓库元数据在复核后已变更；请重新打开并复核";
    RefuseTrustReviewExpired => "trust review expired or changed; open again", "信任复核已过期或已变更；请重新打开";
    HintWorking => "Working; wait for the actual result", "处理中；请等待实际结果";
    HintRemoteReview => "y Apply exactly this   n/Esc Cancel   Page keys Scroll   Enter does not approve", "y 按此应用   n/Esc 取消   Page 滚动   Enter 不会批准";
    HintRemoteForm => "Tab Field   Enter Review   Space fetch/push (Set URL)   Esc Cancel", "Tab 切换字段   Enter 复核   Space 切换 fetch/push（Set URL）   Esc 取消";
    HintRemotesList => "Up/Down Select   a Add   s Set URL   r Rename   d Remove   f Fetch   Esc Close", "Up/Down 选择   a 添加   s 设置 URL   r 重命名   d 移除   f 抓取   Esc 关闭";
    HintLocateSaving => "Saving location; wait for the actual result", "正在保存定位；请等待实际结果";
    HintLocateReview => "y Save location   n/Esc Back   Enter does not approve   Page keys Scroll", "y 保存定位   n/Esc 返回   Enter 不会批准   Page 滚动";
    HintLocateForm => "Enter Review location   Ctrl+A/C/V Edit   Esc Cancel", "Enter 复核定位   Ctrl+A/C/V 编辑   Esc 取消";
    HintToolWorking => "Working; wait for the actual result. A submitted launch cannot be cancelled.", "处理中；请等待实际结果。已提交的启动无法取消。";
    HintToolReview => "y Launch   n/Esc Cancel   Enter does not approve   Page keys Scroll", "y 启动   n/Esc 取消   Enter 不会批准   Page 滚动";
    HintToolForm => "Ctrl+S Save   Ctrl+Enter Review launch   Tab Field   Esc Close", "Ctrl+S 保存   Ctrl+Enter 复核启动   Tab 切换字段   Esc 关闭";
    HintNavigation => "y Archive view and open   n/Esc Cancel   Enter does not approve", "y 归档视图并打开   n/Esc 取消   Enter 不会批准";
    HintFilter => "Enter Apply   Esc Restore   Tab Change pane   Up/Down Preview   Ctrl+A/C/V Edit", "Enter 应用   Esc 还原   Tab 切换面板   Up/Down 预览   Ctrl+A/C/V 编辑";
    HintIdentityWorking => "Working; fields are read-only until completion. Esc cancels a read, not a submitted save.", "处理中；完成前字段只读。Esc 只取消读取，不会取消已提交的保存。";
    HintIdentityForm => "Tab Switch field   Enter Save local/reload   Page/Up/Down Scroll sources   Esc Cancel", "Tab 切换字段   Enter 保存到本地/重新载入   Page/Up/Down 滚动来源   Esc 取消";
    HintHelp => "Esc close help", "Esc 关闭帮助";
    HintConfirm => "y confirm   n keep", "y 确认   n 保留";
    HintPromptGit => "Enter test and save   Esc close   Blank uses PATH", "Enter 测试并保存   Esc 关闭   留空则用 PATH";
    HintPromptCommit => "Ctrl+Enter commit   Enter new line   Esc save draft", "Ctrl+Enter 提交   Enter 换行   Esc 保存草稿";
    HintPromptOpen => "Enter open   Esc cancel   Existing repository trust is checked", "Enter 打开   Esc 取消   会检查已有仓库信任";
    HintPromptBranch => "Enter create   Esc cancel", "Enter 创建   Esc 取消";
    HintPendingCheckout => "No working tree yet: o reviews writing {files} file(s) from HEAD   R Remotes   ? help", "尚无工作树：o 复核从 HEAD 写入 {files} 个文件   R 远端   ? 帮助";
    HintBlocked => "Writes are blocked until this operation is resolved   t Terminal here   R Remotes   ? help", "操作解决前写入被阻止   t 在此打开终端   R 远端   ? 帮助";
    HintPaneStatus => "f fetch   p pull   P push   2 3 4 focus lists", "f 抓取   p 拉取   P 推送   2 3 4 聚焦列表";
    HintPaneFiles => "Space stage/unstage   a all   d discard   s stash   5 stashes   c commit", "Space 暂存/取消暂存   a 全部   d 丢弃   s 贮藏   5 贮藏列表   c 提交";
    HintPaneStashes => "Space/S apply and keep   p pop   d drop   2 files   Enter inspect", "Space/S 应用并保留   p 弹出   d 删除   2 文件   Enter 查看";
    HintPaneBranches => "Space checkout   n new   d delete   Enter details", "Space 检出   n 新建   d 删除   Enter 详情";
    HintPaneCommits => "Enter files   Space checkout detached", "Enter 文件   Space 分离检出";
    HintMainCommitted => "Enter file diff   Esc commits", "Enter 文件差异   Esc 返回提交";
    HintMainScroll => "j/k scroll   Esc back", "j/k 滚动   Esc 返回";
    HintMainAnchor => "Space stage lines   v end range   a stage hunk   Esc back", "Space 暂存所选行   v 结束范围   a 暂存区块   Esc 返回";
    HintMainLines => "Space stage line   v range   a stage hunk   d discard   Esc back", "Space 暂存该行   v 选择范围   a 暂存区块   d 丢弃   Esc 返回";
    HintSuffix => "{focus}   Tab panels   ? help   q quit", "{focus}   Tab 切换面板   ? 帮助   q 退出";
    HintWelcomeFooter => "Language: {label} [y]   C Clone   Ctrl+O Open   N Create   G Git settings   T Tools   Enter Open selected   L Locate   d Remove   q Quit", "语言：{label} [y]   C 克隆   Ctrl+O 打开   N 创建   G Git 设置   T 工具   Enter 打开所选   L 定位   d 移除   q 退出";
    HintCloneFooter => "Tab Switch field   Enter Review   y Clone after review   Esc Back/cancel", "Tab 切换字段   Enter 复核   y 复核后克隆   Esc 返回/取消";
    HintCreateFooter => "Tab Switch field   Enter Review   y Create after review   Esc Back/cancel", "Tab 切换字段   Enter 复核   y 复核后创建   Esc 返回/取消";
    HintRepoFooter => "Ctrl+O Open   N Create   G Git settings   Enter Open selected   L Locate   d Remove   q Quit", "Ctrl+O 打开   N 创建   G Git 设置   Enter 打开所选   L 定位   d 移除   q 退出";
    CloneFailureHint => "i Inspect target   x Remove it   Esc Close", "i 检查目标   x 移除   Esc 关闭";
    CloneRemovalHint => "y Remove   n/Esc Back   Page keys Scroll", "y 移除   n/Esc 返回   Page 键滚动";
    CloneRemoveTitle => "Review removal", "复核移除";
    CloneEntries => "Entries: {shown} of {total}{more}", "条目：显示 {shown}/{total}{more}";
    CloneMore => ", and more", "，还有更多";
    CloneRemovable => "x reviews removing this target. Only this directory and what is inside it is touched.", "x 可复核移除此目标。只会触碰该目录及其内容。";
    CloneNotRemovable => "The directory existed before this clone, so it is not this attempt's to remove. Inspect it and delete it yourself if you want.", "该目录在克隆前已存在，不由本次克隆创建，因此产品不会删除。如需删除请自行处理。";
    NoteCloneFailed => "Clone failed: {reason}", "克隆失败：{reason}";
    NoteCloneTargetRemoved => "The partial clone target was removed; nothing outside it was touched.", "未完成克隆的目标目录已移除；目标之外未做任何改动。";
    NoteCloneNothingToRemove => "There is no recorded clone target to remove.", "没有可移除的克隆目标记录。";
    CloneRemoveTarget => "remove {destination}", "移除 {destination}";
    CloneRemoveListing => "Contents: {shown} of {total}{more}", "内容：显示 {shown}/{total}{more}";
    CloneRemoveEffect => "This deletes the directory and everything under it. Nothing here is recoverable afterwards.", "这将删除该目录及其全部内容，之后无法从此处恢复。";
    NoteCloneReviewExpired => "The failed-clone record expired; clone again to see its target.", "失败克隆的记录已过期；请重新克隆以查看目标。";
    NoteCloneRemovalFailed => "The target was not removed: {reason}", "未移除目标：{reason}";
    NoteBranchCheckedOut => "{branch} is already checked out", "已检出分支 {branch}";
    NoteBranchCannotDelete => "{branch} is checked out and cannot be deleted", "分支 {branch} 已检出，无法删除";
    NoteNothingToDiscard => "{path} has no unstaged changes to discard", "{path} 没有可丢弃的未暂存更改";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_carries_both_strings_and_they_differ() {
        for key in Key::ALL {
            let en = english(*key);
            let zh = chinese(*key);
            assert!(!en.trim().is_empty(), "{key:?} has no English text");
            assert!(!zh.trim().is_empty(), "{key:?} has no Chinese text");
            // A Chinese entry that equals its English one is a missed
            // translation that would pass a mere non-empty check.
            assert_ne!(en, zh, "{key:?} was not translated");
        }
    }

    #[test]
    fn the_catalog_enumeration_matches_the_table() {
        // ALL is what the coverage check and any future export walks; if the
        // macro ever emitted it from a different list this would drift.
        let all = Key::ALL.len();
        assert!(all >= 60, "catalog lost entries: {all}");
        let english_texts: std::collections::BTreeSet<_> =
            Key::ALL.iter().map(|key| english(*key)).collect();
        assert_eq!(english_texts.len(), all, "two keys share English text");
    }

    #[test]
    fn chinese_text_is_not_accented_latin() {
        // Chinese UI strings must actually contain Han characters; a copy-paste
        // of the English column would otherwise look translated to a length
        // check but not to a reader.
        for key in Key::ALL {
            let zh = chinese(*key);
            assert!(
                zh.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                "{key:?} has no Han character: {zh}"
            );
        }
    }

    #[test]
    fn locale_tags_round_trip_and_unknown_values_fall_back() {
        assert_eq!(Locale::from_tag(Locale::En.tag()), Locale::En);
        assert_eq!(Locale::from_tag(Locale::ZhCn.tag()), Locale::ZhCn);
        assert_eq!(Locale::from_tag("zh_CN"), Locale::ZhCn);
        assert_eq!(Locale::from_tag("fr"), Locale::En);
        assert_eq!(Locale::from_tag(""), Locale::En);
    }

    #[test]
    fn an_explicit_choice_wins_over_the_environment() {
        assert_eq!(Locale::resolve(Some("en"), true), Locale::En);
        assert_eq!(Locale::resolve(Some("zh-CN"), false), Locale::ZhCn);
        assert_eq!(Locale::resolve(None, true), Locale::ZhCn);
        assert_eq!(Locale::resolve(None, false), Locale::En);
        // "system" is the stored form of "follow the environment".
        assert_eq!(Locale::resolve(Some("system"), true), Locale::ZhCn);
        assert_eq!(Locale::resolve(Some("system"), false), Locale::En);
    }

    #[test]
    fn text_dispatches_by_locale() {
        assert_eq!(text(Key::ActionOpen, Locale::En), "Open");
        assert_eq!(text(Key::ActionOpen, Locale::ZhCn), "打开");
    }
}
