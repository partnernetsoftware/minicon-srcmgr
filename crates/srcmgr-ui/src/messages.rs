//! Product-owned message keys. English is the delivered fallback in 0.0.6.
//! Keys are semantic Rust symbols, never persisted as enum ordinals. A locale
//! selector and primary-flow translations remain separate delivery work.
//! Static lookup adds no service, parser, allocation or mutable global locale.
macro_rules! catalog {
    ($($key:ident => $text:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub(crate) enum Key { $($key),* }
        pub(crate) const fn english(key: Key) -> &'static str {
            match key { $(Key::$key => $text),* }
        }
    };
}

catalog! {
    ActionOpen => "Open",
    ActionBack => "Back",
    ActionCancel => "Cancel",
    ActionClose => "Close",
    EntryCreate => "Create",
    EntryGitSettings => "Git settings",
    EntryIdentitySettings => "Identity settings",
    EntryOpenTitle => "Open repository",
    EntryRepositoryPath => "Repository path",
    EntryGitExecutable => "Git executable (blank = PATH)",
    EntryApplicationGitSettings => "Application Git settings",
    EntryTestSave => "Test and save",
    EntrySelectionDiagnostics => "Selection and diagnostics",
    EntryWorking => "Working...",
    EntryFinishIme => "Finish IME",
    CreateTitle => "Create local Git repository",
    CreateDirectory => "Directory (parent must exist)",
    CreateBranch => "Initial branch",
    CreateReviewTitle => "Review repository creation",
    CreateReview => "Review creation",
    CreateApprove => "Create repository",
    CreateBefore => "Before creating",
    IdentityTitle => "Repository identity (local scope)",
    IdentityName => "Name",
    IdentityEmail => "Email",
    IdentitySources => "Effective identity and config sources",
    IdentitySourcesScroll => "Effective identity and config sources (scroll)",
    IdentityReload => "Reload preview",
    IdentitySave => "Save local",
    TrustReview => "Repository trust review",
    TrustReviewTitle => "Review repository trust",
    TrustApprove => "Trust and open",
    CommitPrepare => "Prepare commit",
    CommitMessage => "Commit message",
    CommitApprove => "Commit",
    CommitDisabled => "Commit disabled",
    CommitRefresh => "Refresh review",
    CommitClose => "Save and close",
    CommitIndex => "Complete staged index",
    CommitDiff => "Staged Diff",
    ChangesStage => "Stage",
    ChangesUnstage => "Unstage",
    ChangesDiscard => "Discard",
    ChangesIgnore => "Ignore",
    ChangesProjectIgnore => "Project .gitignore",
    ChangesLocalExclude => "Local exclude",
    ChangesSaveRule => "Save rule",
    ChangesConfirm => "Confirm reviewed action",
    ChangesReview => "Action review",
    ChangesVisibleDiff => "Selected Diff visible content",
    CurrentActionStatus => "Current action status",
    RecentOpen => "Open selected",
    RecentLocate => "Locate recent",
    RecentTitle => "Recent repositories",
    RecentEmpty => "No recent repositories",
    WelcomeTitle => "Welcome",
    BranchNewTitle => "New branch name",
    FilterClear => "Clear",
    LocateReview => "Review location",
    LocateSave => "Save location",
    RecoveryArchiveOpen => "Archive view and open",
    ToolsSave => "Save",
    ToolsReview => "Review launch",
}
