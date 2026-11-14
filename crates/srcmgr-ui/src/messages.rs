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
        pub(crate) enum Key { $($key),* }

        pub(crate) const fn english(key: Key) -> &'static str {
            match key { $(Key::$key => $en),* }
        }

        pub(crate) const fn chinese(key: Key) -> &'static str {
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

pub(crate) const fn text(key: Key, locale: Locale) -> &'static str {
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
