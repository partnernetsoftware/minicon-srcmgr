//! Local projections of loaded rows; indices never replace raw action identity.
#[derive(Default)]
pub(crate) struct FilterIndex {
    pub query: String,
    labels: Vec<String>,
    revisions: Vec<String>,
    pub rows: Vec<usize>,
}

impl FilterIndex {
    pub fn load(&mut self, labels: impl Iterator<Item = String>, revisions: impl Iterator<Item = String>) {
        self.labels = labels.map(|s| s.to_lowercase()).collect();
        self.revisions = revisions.map(|s| s.to_ascii_lowercase()).collect();
        self.project();
    }

    pub fn project(&mut self) {
        let query = self.query.to_lowercase();
        let revision_prefix = !query.is_empty() && query.bytes().all(|b| b.is_ascii_hexdigit());
        self.rows.clear();
        self.rows.extend(self.labels.iter().enumerate().filter_map(|(i, label)|
            (label.contains(&query) || (revision_prefix && self.revisions.get(i).is_some_and(|id| id.starts_with(&query)))).then_some(i)));
    }
}
