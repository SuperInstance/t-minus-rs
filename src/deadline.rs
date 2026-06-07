//! Deadline propagation tree: parent deadline distributes to children,
//! cancellation cascades on parent expiry, deadline inheritance.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Status of a deadline node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlineStatus {
    Active,
    Expired,
    Cancelled,
}

/// A node in the deadline propagation tree.
pub struct DeadlineNode {
    inner: Arc<Mutex<DeadlineInner>>,
}

#[derive(Debug)]
struct DeadlineInner {
    id: u64,
    deadline: Option<Instant>,
    status: DeadlineStatus,
    children: Vec<Arc<Mutex<DeadlineInner>>>,
    #[allow(dead_code)]
    parent_id: Option<u64>,
}

impl DeadlineNode {
    /// Create a new root deadline node with an optional deadline duration from now.
    pub fn new(id: u64, deadline: Option<Duration>) -> Self {
        let inner = DeadlineInner {
            id,
            deadline: deadline.map(|d| Instant::now() + d),
            status: DeadlineStatus::Active,
            children: Vec::new(),
            parent_id: None,
        };
        DeadlineNode {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Get the node ID.
    pub fn id(&self) -> u64 {
        self.inner.lock().unwrap().id
    }

    /// Check the current status, potentially updating to Expired if deadline passed.
    pub fn status(&self) -> DeadlineStatus {
        let mut inner = self.inner.lock().unwrap();
        if inner.status == DeadlineStatus::Active {
            if let Some(dl) = inner.deadline {
                if Instant::now() >= dl {
                    inner.status = DeadlineStatus::Expired;
                }
            }
        }
        inner.status
    }

    /// Get remaining time until deadline, or None if no deadline / already expired.
    pub fn remaining(&self) -> Option<Duration> {
        let inner = self.inner.lock().unwrap();
        match inner.deadline {
            Some(dl) if inner.status == DeadlineStatus::Active => {
                let now = Instant::now();
                if now >= dl {
                    None
                } else {
                    Some(dl - now)
                }
            }
            _ => None,
        }
    }

    /// Manually cancel this node and cascade cancellation to all children.
    pub fn cancel(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.status != DeadlineStatus::Active {
            return;
        }
        inner.status = DeadlineStatus::Cancelled;
        let children: Vec<_> = inner.children.clone();
        drop(inner);
        cascade_cancel(&children);
    }

    /// Check if deadline has expired and cascade expiry to children.
    pub fn check_and_propagate(&self) -> DeadlineStatus {
        let mut inner = self.inner.lock().unwrap();
        if inner.status == DeadlineStatus::Active {
            if let Some(dl) = inner.deadline {
                if Instant::now() >= dl {
                    inner.status = DeadlineStatus::Expired;
                    let children: Vec<_> = inner.children.clone();
                    drop(inner);
                    cascade_expire(&children);
                    return DeadlineStatus::Expired;
                }
            }
        }
        inner.status
    }

    /// Add a child node that inherits the parent's deadline (tightened if child's is sooner).
    /// Returns the child DeadlineNode.
    pub fn add_child(&self, child_id: u64, child_deadline: Option<Duration>) -> DeadlineNode {
        let mut inner = self.inner.lock().unwrap();
        let parent_dl = inner.deadline;
        let parent_status = inner.status;

        // Child inherits parent deadline if it's tighter
        let effective_deadline = match (child_deadline, parent_dl) {
            (Some(cd), Some(pd)) => {
                let child_abs = Instant::now() + cd;
                Some(if child_abs < pd { child_abs } else { pd })
            }
            (None, Some(pd)) => Some(pd),
            (Some(cd), None) => Some(Instant::now() + cd),
            (None, None) => None,
        };

        let child_inner = Arc::new(Mutex::new(DeadlineInner {
            id: child_id,
            deadline: effective_deadline,
            status: parent_status, // Inherit parent status
            children: Vec::new(),
            parent_id: Some(inner.id),
        }));

        inner.children.push(Arc::clone(&child_inner));
        drop(inner);

        DeadlineNode { inner: child_inner }
    }

    /// Get number of children.
    pub fn child_count(&self) -> usize {
        self.inner.lock().unwrap().children.len()
    }

    /// Get the absolute deadline instant (for testing).
    pub fn deadline_instant(&self) -> Option<Instant> {
        self.inner.lock().unwrap().deadline
    }
}

fn cascade_cancel(children: &[Arc<Mutex<DeadlineInner>>]) {
    for child in children {
        let mut c = child.lock().unwrap();
        if c.status == DeadlineStatus::Active {
            c.status = DeadlineStatus::Cancelled;
            let grandchildren: Vec<_> = c.children.clone();
            drop(c);
            cascade_cancel(&grandchildren);
        }
    }
}

fn cascade_expire(children: &[Arc<Mutex<DeadlineInner>>]) {
    for child in children {
        let mut c = child.lock().unwrap();
        if c.status == DeadlineStatus::Active {
            c.status = DeadlineStatus::Expired;
            let grandchildren: Vec<_> = c.children.clone();
            drop(c);
            cascade_expire(&grandchildren);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn new_node_is_active() {
        let node = DeadlineNode::new(1, Some(Duration::from_secs(60)));
        assert_eq!(node.status(), DeadlineStatus::Active);
        assert_eq!(node.id(), 1);
    }

    #[test]
    fn node_expires_after_deadline() {
        let node = DeadlineNode::new(1, Some(Duration::from_millis(10)));
        thread::sleep(Duration::from_millis(20));
        assert_eq!(node.status(), DeadlineStatus::Expired);
    }

    #[test]
    fn node_with_no_deadline_stays_active() {
        let node = DeadlineNode::new(1, None);
        assert_eq!(node.status(), DeadlineStatus::Active);
        assert!(node.remaining().is_none());
    }

    #[test]
    fn cancel_sets_status() {
        let node = DeadlineNode::new(1, Some(Duration::from_secs(60)));
        node.cancel();
        assert_eq!(node.status(), DeadlineStatus::Cancelled);
    }

    #[test]
    fn cancel_cascades_to_children() {
        let parent = DeadlineNode::new(1, Some(Duration::from_secs(60)));
        let child = parent.add_child(2, Some(Duration::from_secs(120)));
        let grandchild = child.add_child(3, None);
        parent.cancel();
        assert_eq!(parent.status(), DeadlineStatus::Cancelled);
        assert_eq!(child.status(), DeadlineStatus::Cancelled);
        assert_eq!(grandchild.status(), DeadlineStatus::Cancelled);
    }

    #[test]
    fn child_inherits_parent_deadline() {
        let parent = DeadlineNode::new(1, Some(Duration::from_secs(10)));
        let child = parent.add_child(2, Some(Duration::from_secs(120)));
        // Child's 120s should be tightened to parent's 10s
        let parent_dl = parent.deadline_instant().unwrap();
        let child_dl = child.deadline_instant().unwrap();
        assert!(child_dl <= parent_dl + Duration::from_millis(50));
    }

    #[test]
    fn child_keeps_own_deadline_if_tighter() {
        let parent = DeadlineNode::new(1, Some(Duration::from_secs(120)));
        let child = parent.add_child(2, Some(Duration::from_millis(10)));
        thread::sleep(Duration::from_millis(20));
        // Child's tighter deadline should expire
        assert_eq!(child.status(), DeadlineStatus::Expired);
        // Parent should still be active
        assert_eq!(parent.status(), DeadlineStatus::Active);
    }

    #[test]
    fn child_inherits_no_deadline_from_parent() {
        let parent = DeadlineNode::new(1, None);
        let child = parent.add_child(2, None);
        assert_eq!(child.status(), DeadlineStatus::Active);
        assert!(child.remaining().is_none());
    }

    #[test]
    fn expire_propagates_to_children() {
        let parent = DeadlineNode::new(1, Some(Duration::from_millis(10)));
        let child = parent.add_child(2, Some(Duration::from_secs(60)));
        thread::sleep(Duration::from_millis(20));
        let status = parent.check_and_propagate();
        assert_eq!(status, DeadlineStatus::Expired);
        assert_eq!(child.status(), DeadlineStatus::Expired);
    }

    #[test]
    fn remaining_decreases_over_time() {
        let node = DeadlineNode::new(1, Some(Duration::from_millis(100)));
        let r1 = node.remaining().unwrap();
        thread::sleep(Duration::from_millis(30));
        let r2 = node.remaining().unwrap();
        assert!(r2 < r1);
    }

    #[test]
    fn child_count_tracks_additions() {
        let parent = DeadlineNode::new(1, Some(Duration::from_secs(60)));
        assert_eq!(parent.child_count(), 0);
        parent.add_child(2, None);
        assert_eq!(parent.child_count(), 1);
        parent.add_child(3, None);
        assert_eq!(parent.child_count(), 2);
    }

    #[test]
    fn double_cancel_is_noop() {
        let node = DeadlineNode::new(1, Some(Duration::from_secs(60)));
        node.cancel();
        node.cancel(); // Should not panic
        assert_eq!(node.status(), DeadlineStatus::Cancelled);
    }

    #[test]
    fn expired_child_does_not_affect_parent() {
        let parent = DeadlineNode::new(1, Some(Duration::from_secs(60)));
        let child = parent.add_child(2, Some(Duration::from_millis(10)));
        thread::sleep(Duration::from_millis(20));
        assert_eq!(child.status(), DeadlineStatus::Expired);
        assert_eq!(parent.status(), DeadlineStatus::Active);
    }
}
