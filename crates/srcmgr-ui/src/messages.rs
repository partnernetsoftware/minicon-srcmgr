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
