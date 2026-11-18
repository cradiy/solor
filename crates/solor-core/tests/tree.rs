//! Tree ownership, identity and structural edits.
use std::cell::Cell;
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use solor_core::tree::{NodeId, Tree, TreeError};

fn assert_topology<T>(tree: &Tree<T>) {
    assert_eq!(tree.parent(tree.root()), Ok(None));
    let mut seen = HashSet::new();
    let mut pending = vec![tree.root()];
    while let Some(id) = pending.pop() {
        assert!(seen.insert(id), "duplicate node or cycle");
        assert!(tree.get(id).is_some());
        for child in tree.children(id).unwrap() {
            assert_eq!(tree.parent(child), Ok(Some(id)));
            pending.push(child);
        }
    }
    assert_eq!(seen.len(), tree.node_count(), "unreachable live nodes");
}

#[test]
fn handles_survive_growth_and_payload_edits_are_local() {
    let mut tree = Tree::new(0);
    let root = tree.root();
    let ids: Vec<_> = (1..10_000)
        .map(|value| tree.append(root, value).unwrap())
        .collect();
    let changed = 4_321;
    *tree.get_mut(ids[changed]).unwrap() = usize::MAX;
    for (index, &id) in ids.iter().enumerate() {
        let expected = if index == changed {
            usize::MAX
        } else {
            index + 1
        };
        assert_eq!(tree.get(id), Some(&expected));
    }
    assert_eq!(tree.node_count(), 10_000);
    assert_eq!(tree.children(root).unwrap().collect::<Vec<_>>(), ids);
    assert_topology(&tree);
}

#[test]
fn subtree_removal_preserves_survivor_identity_and_order() {
    let mut tree = Tree::new("root");
    let root = tree.root();
    let before = tree.append(root, "before").unwrap();
    let branch = tree.append(root, "branch").unwrap();
    let after = tree.append(root, "after").unwrap();
    let first = tree.append(branch, "first").unwrap();
    let second = tree.append(branch, "second").unwrap();
    let deep = tree.append(first, "deep").unwrap();

    assert_eq!(tree.remove_subtree(branch), Ok(4));
    for id in [branch, first, second, deep] {
        assert_eq!(tree.get(id), None);
        assert_eq!(tree.get_mut(id), None);
        assert_eq!(tree.parent(id), Err(TreeError::InvalidNode(id)));
    }
    assert_eq!(
        tree.children(root).unwrap().collect::<Vec<_>>(),
        [before, after]
    );
    assert_eq!(tree.get(before), Some(&"before"));
    assert_eq!(tree.get(after), Some(&"after"));

    let replacement = tree.append(root, "replacement").unwrap();
    assert_eq!(
        tree.children(root).unwrap().rev().collect::<Vec<_>>(),
        [replacement, after, before]
    );
    assert_topology(&tree);
}

#[test]
fn stale_and_foreign_handles_are_rejected_without_structural_changes() {
    let mut tree = Tree::new(0);
    let root = tree.root();
    let stale = tree.append(root, 1).unwrap();
    tree.remove_subtree(stale).unwrap();
    let other = Tree::new(2);
    let foreign = other.root();
    for _ in 0..1_000 {
        let live = tree.append(root, 3).unwrap();
        assert_ne!(live, stale);
        for invalid in [stale, foreign] {
            assert_eq!(tree.get(invalid), None);
            assert_eq!(tree.get_mut(invalid), None);
            assert_eq!(tree.parent(invalid), Err(TreeError::InvalidNode(invalid)));
            assert!(
                matches!(tree.children(invalid), Err(TreeError::InvalidNode(id)) if id == invalid)
            );
            assert_eq!(
                tree.append(invalid, 4),
                Err(TreeError::InvalidNode(invalid))
            );
            assert_eq!(
                tree.remove_subtree(invalid),
                Err(TreeError::InvalidNode(invalid))
            );
        }
        assert_eq!(tree.remove_subtree(root), Err(TreeError::CannotRemoveRoot));
        assert_eq!(tree.node_count(), 2);
        assert_eq!(tree.children(root).unwrap().collect::<Vec<_>>(), [live]);
        tree.remove_subtree(live).unwrap();
    }
    assert_topology(&tree);
}

struct CountDrop(Rc<Cell<usize>>);

impl Drop for CountDrop {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn deep_removal_and_tree_drop_release_payloads_without_topology_recursion() {
    let drops = Rc::new(Cell::new(0));
    let mut tree = Tree::new(CountDrop(drops.clone()));
    let branch = tree.append(tree.root(), CountDrop(drops.clone())).unwrap();
    let mut parent = branch;
    for _ in 1..50_000 {
        parent = tree.append(parent, CountDrop(drops.clone())).unwrap();
    }
    assert_eq!(tree.remove_subtree(branch), Ok(50_000));
    assert_eq!(drops.get(), 50_000);
    assert_eq!(tree.node_count(), 1);
    assert_topology(&tree);

    parent = tree.root();
    for _ in 0..50_000 {
        parent = tree.append(parent, CountDrop(drops.clone())).unwrap();
    }
    drop(tree);
    assert_eq!(drops.get(), 100_001);
}

#[test]
fn panicking_payload_drop_leaves_valid_remaining_topology() {
    struct Payload(bool);
    impl Drop for Payload {
        fn drop(&mut self) {
            assert!(!self.0, "intentional destructor panic");
        }
    }
    let mut tree = Tree::new(Payload(false));
    let branch = tree.append(tree.root(), Payload(false)).unwrap();
    let survivor = tree.append(branch, Payload(false)).unwrap();
    let bomb = tree.append(branch, Payload(true)).unwrap();
    assert!(catch_unwind(AssertUnwindSafe(|| tree.remove_subtree(branch))).is_err());
    assert!(tree.get(bomb).is_none());
    assert!(tree.get(survivor).is_some());
    assert_eq!(tree.node_count(), 3);
    assert_topology(&tree);
    assert_eq!(tree.remove_subtree(branch), Ok(2));
    assert_topology(&tree);
}

#[test]
fn mixed_edits_match_a_parent_table_model() {
    let mut tree = Tree::new(0);
    let mut model: Vec<(NodeId, Option<NodeId>, usize)> = vec![(tree.root(), None, 0)];
    let mut seed = 17_u64;
    for value in 1..600 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let index = (seed >> 32) as usize % model.len();
        let id = model[index].0;
        match seed % 5 {
            0 if index != 0 => {
                let mut removed = HashSet::from([id]);
                for &(candidate, parent, _) in &model {
                    if parent.is_some_and(|parent| removed.contains(&parent)) {
                        removed.insert(candidate);
                    }
                }
                assert_eq!(tree.remove_subtree(id), Ok(removed.len()));
                model.retain(|(candidate, _, _)| !removed.contains(candidate));
                for stale in removed {
                    assert!(tree.get(stale).is_none());
                }
            }
            1 => {
                *tree.get_mut(id).unwrap() = value;
                model[index].2 = value;
            }
            _ => {
                let child = tree.append(id, value).unwrap();
                model.push((child, Some(id), value));
            }
        }
        assert_eq!(tree.node_count(), model.len());
        for &(id, parent, payload) in &model {
            assert_eq!(tree.get(id), Some(&payload));
            assert_eq!(tree.parent(id), Ok(parent));
            let expected: Vec<_> = model
                .iter()
                .filter(|(_, parent, _)| *parent == Some(id))
                .map(|&(child, _, _)| child)
                .collect();
            assert_eq!(tree.children(id).unwrap().collect::<Vec<_>>(), expected);
        }
        assert_topology(&tree);
    }
}
