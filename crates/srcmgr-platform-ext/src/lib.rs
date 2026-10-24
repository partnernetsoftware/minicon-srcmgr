//! Private, bounded publication adapter. VCS meaning remains in the UI model.
pub use minicon_platform::accessibility_publish::{
    AccessibilityBounds, PublishedNode, PublishedRole, PublishedTree,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub const MAX_NODES: usize = 512;
pub const MAX_TEXT: usize = 1024 * 1024;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub revision: u64,
    pub context: u64,
    pub tree: PublishedTree,
    pub disabled: Vec<u32>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Invoke,
    Focus,
    SetValue(String),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub revision: u64,
    pub context: u64,
    pub name: String,
    pub node: u32,
    pub action: Action,
}
#[derive(Debug)]
pub enum Error {
    Unsupported,
    InvalidSnapshot,
    Native(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

impl Snapshot {
    pub fn validate(&self) -> Result<(), Error> {
        let nodes = &self.tree.nodes;
        if nodes.is_empty() || nodes.len() > MAX_NODES || self.revision == 0 {
            return Err(Error::InvalidSnapshot);
        }
        let mut bytes = self.tree.app_name.len();
        let mut roots = 0;
        for (i, node) in nodes.iter().enumerate() {
            bytes = bytes
                .saturating_add(node.name.len())
                .saturating_add(node.text.len());
            if bytes > MAX_TEXT
                || nodes[..i].iter().any(|other| other.id == node.id)
                || node.bounds.width < 0
                || node.bounds.height < 0
            {
                return Err(Error::InvalidSnapshot);
            }
            if node.parent.is_none() {
                roots += 1;
            }
            let mut parent = node.parent;
            for depth in 0..=nodes.len() {
                let Some(id) = parent else {
                    break;
                };
                if id == node.id || depth >= 32 {
                    return Err(Error::InvalidSnapshot);
                }
                parent = nodes
                    .iter()
                    .find(|n| n.id == id)
                    .ok_or(Error::InvalidSnapshot)?
                    .parent;
            }
        }
        if roots != 1
            || nodes[0].id != 1
            || nodes[0].parent.is_some()
            || self
                .disabled
                .iter()
                .any(|id| !nodes.iter().any(|n| n.id == *id))
        {
            return Err(Error::InvalidSnapshot);
        }
        Ok(())
    }
}

struct Shared {
    snapshot: Mutex<Option<Arc<Snapshot>>>,
    queue: Mutex<VecDeque<Request>>,
    accepting: AtomicBool,
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
}
impl Shared {
    #[cfg(any(windows, test))]
    fn request(&self, revision: u64, node: u32, action: Action) -> bool {
        if !self.accepting.load(Ordering::Acquire) {
            return false;
        }
        let Ok(guard) = self.snapshot.lock() else {
            return false;
        };
        let Some(snapshot) = guard.as_ref().filter(|s| s.revision == revision) else {
            return false;
        };
        let Some(target) = snapshot.tree.node(node) else {
            return false;
        };
        if snapshot.disabled.contains(&node)
            || match &action {
                Action::Invoke => !target.clickable,
                Action::Focus => !target.focusable,
                Action::SetValue(text) => !target.editable || text.len() > MAX_TEXT,
            }
        {
            return false;
        }
        let Ok(mut queue) = self.queue.lock() else {
            return false;
        };
        let bytes = |a: &Action| {
            if let Action::SetValue(s) = a {
                s.len()
            } else {
                0
            }
        };
        if queue.len() >= 16
            || queue.iter().map(|r| bytes(&r.action)).sum::<usize>() + bytes(&action) > MAX_TEXT
        {
            return false;
        }
        queue.push_back(Request {
            revision,
            context: snapshot.context,
            name: target.name.clone(),
            node,
            action,
        });
        // Wake posts an asynchronous host message. Keep queue ownership until
        // its verdict so failed posting cannot leave executable authority.
        if !(self.wake)() {
            queue.pop_back();
            return false;
        }
        true
    }
}

pub struct Publisher {
    shared: Arc<Shared>,
    #[cfg(windows)]
    native: windows_uia::Attachment,
}
impl Publisher {
    pub fn attach(
        identity: Option<i64>,
        title: &str,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Self, Error> {
        #[cfg(not(windows))]
        {
            let _ = (identity, title, wake);
            Err(Error::Unsupported)
        }
        #[cfg(windows)]
        {
            let shared = Arc::new(Shared {
                snapshot: Mutex::new(None),
                queue: Mutex::new(VecDeque::new()),
                accepting: AtomicBool::new(false),
                wake,
            });
            let native = windows_uia::Attachment::new(identity, title, shared.clone())?;
            Ok(Self { shared, native })
        }
    }
    pub fn publish(&self, snapshot: Snapshot) -> Result<(), Error> {
        snapshot.validate()?;
        *self
            .shared
            .snapshot
            .lock()
            .map_err(|_| Error::InvalidSnapshot)? = Some(Arc::new(snapshot));
        self.shared.accepting.store(true, Ordering::Release);
        #[cfg(windows)]
        self.native.changed();
        Ok(())
    }
    pub fn requests(&self) -> Vec<Request> {
        self.shared
            .queue
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }
    /// Keep readable frame data until atomic replacement; revoke new actions.
    /// Already queued requests are revalidated against live UI context.
    pub fn invalidate(&self) {
        self.shared.accepting.store(false, Ordering::Release);
    }
    pub fn focus(&self) {
        #[cfg(windows)]
        self.native.focus();
    }
}
#[cfg(windows)]
mod windows_uia;

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        Snapshot {
            revision: 1,
            context: 1,
            tree: PublishedTree {
                app_name: "Fixture".into(),
                nodes: vec![PublishedNode {
                    id: 1,
                    parent: None,
                    role: PublishedRole::Button,
                    name: "Reviewed action".into(),
                    text: String::new(),
                    bounds: AccessibilityBounds {
                        x: 0,
                        y: 0,
                        width: 40,
                        height: 20,
                    },
                    focusable: true,
                    focused: false,
                    editable: false,
                    clickable: true,
                }],
            },
            disabled: vec![],
        }
    }
    #[test]
    fn bounded_tree_and_requests_refuse_stale_disabled_and_unbounded_work() {
        let shared = Shared {
            snapshot: Mutex::new(Some(Arc::new(snapshot()))),
            queue: Mutex::new(VecDeque::new()),
            accepting: AtomicBool::new(true),
            wake: Arc::new(|| true),
        };
        assert!(shared.request(1, 1, Action::Invoke));
        assert_eq!(
            shared.queue.lock().unwrap().pop_front().unwrap(),
            Request {
                revision: 1,
                context: 1,
                name: "Reviewed action".into(),
                node: 1,
                action: Action::Invoke
            }
        );
        assert!(!shared.request(2, 1, Action::Invoke));
        assert!(!shared.request(1, 1, Action::SetValue("uneditable".into())));
        let mut changed = snapshot();
        changed.disabled.push(1);
        *shared.snapshot.lock().unwrap() = Some(Arc::new(changed));
        assert!(!shared.request(1, 1, Action::Invoke));
        assert!(shared.queue.lock().unwrap().is_empty());
        let mut invalid = snapshot();
        invalid.tree.nodes[0].parent = Some(1);
        assert!(invalid.validate().is_err());
        invalid = snapshot();
        invalid.tree.nodes[0].text = "x".repeat(MAX_TEXT + 1);
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn failed_wake_leaves_no_executable_request_and_queue_cost_is_bounded() {
        let shared = Shared {
            snapshot: Mutex::new(Some(Arc::new(snapshot()))),
            queue: Mutex::new(VecDeque::new()),
            accepting: AtomicBool::new(true),
            wake: Arc::new(|| false),
        };
        assert!(!shared.request(1, 1, Action::Invoke));
        assert!(
            shared.queue.lock().unwrap().is_empty(),
            "a failed wake must not execute later under a different event"
        );
        let mut shared = shared;
        shared.wake = Arc::new(|| true);
        for _ in 0..16 {
            assert!(shared.request(1, 1, Action::Invoke));
        }
        assert!(!shared.request(1, 1, Action::Invoke));
        assert_eq!(shared.queue.lock().unwrap().len(), 16);
        shared.queue.lock().unwrap().clear();
        let mut editable = snapshot();
        editable.tree.nodes[0].editable = true;
        *shared.snapshot.lock().unwrap() = Some(Arc::new(editable));
        assert!(shared.request(1, 1, Action::SetValue("x".repeat(MAX_TEXT))));
        assert!(!shared.request(1, 1, Action::SetValue("y".into())));
        assert_eq!(shared.queue.lock().unwrap().len(), 1);
    }
}
