use std::cell::Cell;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use solor_core::tree::TreeError;

use super::store::{AccessError, Context, NodeStore, Status};

fn limit(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap()
}

trait Behavior {
    fn run(&mut self, ctx: &mut Context<'_, dyn Behavior>) -> Result<u64, AccessError>;
    fn visits(&self) -> usize;
}

struct Container {
    visits: usize,
}
struct Leaf {
    value: u64,
    visits: usize,
}

impl Behavior for Container {
    fn run(&mut self, ctx: &mut Context<'_, dyn Behavior>) -> Result<u64, AccessError> {
        self.visits += 1;
        let mut sum = 0;
        for index in 0..ctx.child_count() {
            sum += ctx.execute_child(ctx.child(index).unwrap(), |child, ctx| child.run(ctx))?;
        }
        Ok(sum)
    }
    fn visits(&self) -> usize {
        self.visits
    }
}

impl Behavior for Leaf {
    fn run(&mut self, ctx: &mut Context<'_, dyn Behavior>) -> Result<u64, AccessError> {
        self.visits += 1;
        assert_eq!(ctx.status(ctx.current()), Ok(Status::Running));
        assert_eq!(ctx.status(ctx.parent().unwrap()), Ok(Status::Running));
        Ok(self.value)
    }
    fn visits(&self) -> usize {
        self.visits
    }
}

#[test]
fn heterogeneous_parent_and_child_behaviors_execute_with_stable_addresses() {
    let mut store = NodeStore::<dyn Behavior>::new(Box::new(Container { visits: 0 }), limit(8));
    let root = store.root();
    let branch = store
        .append(root, Box::new(Container { visits: 0 }))
        .unwrap();
    let a = store
        .append(
            branch,
            Box::new(Leaf {
                value: 3,
                visits: 0,
            }),
        )
        .unwrap();
    let b = store
        .append(
            root,
            Box::new(Leaf {
                value: 7,
                visits: 0,
            }),
        )
        .unwrap();
    let ids = [root, branch, a, b];
    let addresses: Vec<_> = ids
        .iter()
        .map(|&id| store.get(id).unwrap() as *const dyn Behavior)
        .collect();
    assert_eq!(store.execute(root, |body, ctx| body.run(ctx)), Ok(10));
    for (&id, &address) in ids.iter().zip(&addresses) {
        assert!(std::ptr::eq(store.get(id).unwrap(), address));
        assert_eq!(store.get(id).unwrap().visits(), 1);
        assert_eq!(store.status(id), Ok(Status::Ready));
    }
}

#[test]
fn reentry_siblings_and_skipped_generations_are_rejected_before_execution() {
    let mut store = NodeStore::new(Box::new(0), limit(8));
    let root = store.root();
    let child = store.append(root, Box::new(0)).unwrap();
    let sibling = store.append(root, Box::new(0)).unwrap();
    let grandchild = store.append(child, Box::new(0)).unwrap();
    store
        .execute(root, |_, ctx| {
            assert_eq!(
                ctx.execute_child(root, |_, _| panic!("must not run")),
                Err::<(), _>(AccessError::ActiveNode(root))
            );
            assert_eq!(
                ctx.execute_child(grandchild, |_, _| panic!("must not run")),
                Err::<(), _>(AccessError::NotChild {
                    parent: root,
                    node: grandchild
                })
            );
            ctx.execute_child(child, |value, ctx| {
                *value += 1;
                assert_eq!(
                    ctx.execute_child(root, |_, _| panic!("must not run")),
                    Err::<(), _>(AccessError::ActiveNode(root))
                );
                assert_eq!(
                    ctx.execute_child(child, |_, _| panic!("must not run")),
                    Err::<(), _>(AccessError::ActiveNode(child))
                );
                assert_eq!(
                    ctx.execute_child(sibling, |_, _| panic!("must not run")),
                    Err::<(), _>(AccessError::NotChild {
                        parent: child,
                        node: sibling
                    })
                );
                Ok(())
            })
        })
        .unwrap();
    assert_eq!(store.get(child), Ok(&1));
    assert_eq!(store.get(sibling), Ok(&0));
    assert_eq!(store.get(grandchild), Ok(&0));
    assert_eq!(store.status(root), Ok(Status::Ready));
}

#[test]
fn stale_and_foreign_handles_do_not_change_execution_state() {
    let mut store = NodeStore::new(Box::new(0), limit(4));
    let root = store.root();
    let stale = store.append(root, Box::new(1)).unwrap();
    store.remove_subtree(stale).unwrap();
    let replacement = store.append(root, Box::new(2)).unwrap();
    let other = NodeStore::new(Box::new(0), limit(4));
    for invalid in [stale, other.root()] {
        let error = AccessError::Tree(TreeError::InvalidNode(invalid));
        assert_eq!(store.get(invalid), Err(error));
        assert_eq!(store.get_mut(invalid), Err(error));
        assert_eq!(store.status(invalid), Err(error));
        assert_eq!(
            store.execute(invalid, |_, _| panic!("must not run")),
            Err::<(), _>(error)
        );
        store
            .execute(root, |_, ctx| {
                assert_eq!(
                    ctx.execute_child(invalid, |_, _| panic!("must not run")),
                    Err::<(), _>(error)
                );
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(store.remove_subtree(root), Err(TreeError::CannotRemoveRoot));
    assert_eq!(store.node_count(), 2);
    assert_eq!(store.get(replacement), Ok(&2));
    assert_eq!(store.status(root), Ok(Status::Ready));
}

struct DropCount {
    value: u64,
    drops: Rc<Cell<usize>>,
}
impl Drop for DropCount {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn panic_restores_all_active_payloads_without_rolling_back_mutations() {
    let drops = Rc::new(Cell::new(0));
    let body = || {
        Box::new(DropCount {
            value: 0,
            drops: drops.clone(),
        })
    };
    let mut store = NodeStore::new(body(), limit(8));
    let root = store.root();
    let child = store.append(root, body()).unwrap();
    let leaf = store.append(child, body()).unwrap();
    let addresses: Vec<_> = [root, child, leaf]
        .iter()
        .map(|&id| store.get(id).unwrap() as *const DropCount)
        .collect();
    let result = catch_unwind(AssertUnwindSafe(|| {
        store.execute::<()>(root, |body, ctx| {
            body.value += 1;
            ctx.execute_child(child, |body, ctx| {
                body.value += 1;
                ctx.execute_child(leaf, |body, _| {
                    body.value += 1;
                    panic!("intentional leaf panic");
                })
            })
        })
    }));
    assert!(result.is_err());
    assert_eq!(drops.get(), 0);
    for (&id, &address) in [root, child, leaf].iter().zip(&addresses) {
        assert!(std::ptr::eq(store.get(id).unwrap(), address));
        assert_eq!(store.get(id).unwrap().value, 1);
        assert_eq!(store.status(id), Ok(Status::Interrupted));
    }
    for id in [root, child, leaf] {
        store
            .execute(id, |body, _| {
                body.value += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(store.status(id), Ok(Status::Ready));
    }
    assert_eq!(store.remove_subtree(child), Ok(2));
    assert_eq!(drops.get(), 2);
    drop(store);
    assert_eq!(drops.get(), 3);
}

#[test]
fn parent_can_catch_child_panic_and_continue_with_a_sibling() {
    let mut store = NodeStore::new(Box::new(0), limit(8));
    let root = store.root();
    let child = store.append(root, Box::new(0)).unwrap();
    let sibling = store.append(root, Box::new(0)).unwrap();
    store
        .execute(root, |value, ctx| {
            let result = catch_unwind(AssertUnwindSafe(|| {
                ctx.execute_child::<()>(child, |_, _| panic!("child panic"))
            }));
            assert!(result.is_err());
            assert_eq!(ctx.status(child), Ok(Status::Interrupted));
            ctx.execute_child(sibling, |value, _| {
                *value += 1;
                Ok(())
            })?;
            *value += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.status(root), Ok(Status::Ready));
    assert_eq!(store.status(child), Ok(Status::Interrupted));
    assert_eq!(store.status(sibling), Ok(Status::Ready));
    assert_eq!(store.get(sibling), Ok(&1));
}

fn descend(value: &mut usize, ctx: &mut Context<'_, usize>) -> Result<(), AccessError> {
    *value += 1;
    if let Some(child) = ctx.child(0) {
        ctx.execute_child(child, descend)?;
    }
    Ok(())
}

#[test]
fn depth_limit_returns_an_error_and_restores_every_entered_node() {
    let mut store = NodeStore::new(Box::new(0_usize), limit(8));
    let root = store.root();
    let mut ids = vec![root];
    for _ in 0..255 {
        ids.push(store.append(*ids.last().unwrap(), Box::new(0)).unwrap());
    }
    assert_eq!(
        store.execute(root, descend),
        Err(AccessError::DepthLimit { limit: 8 })
    );
    for (index, &id) in ids.iter().enumerate() {
        assert_eq!(
            store.status(id),
            Ok(if index < 8 {
                Status::Interrupted
            } else {
                Status::Ready
            })
        );
        assert_eq!(store.get(id), Ok(&usize::from(index < 8)));
    }
    assert_eq!(store.remove_subtree(ids[1]), Ok(255));
    store.execute(root, descend).unwrap();
    assert_eq!(store.status(root), Ok(Status::Ready));
}

#[test]
fn error_return_and_later_retry_preserve_identity_and_payload() {
    let mut store = NodeStore::new(Box::new(0), limit(2));
    let root = store.root();
    assert_eq!(
        store.execute(root, |value, ctx| {
            *value = 9;
            ctx.execute_child(root, |_, _| Ok(()))
        }),
        Err(AccessError::ActiveNode(root))
    );
    assert_eq!(store.get(root), Ok(&9));
    assert_eq!(store.status(root), Ok(Status::Interrupted));
    *store.get_mut(root).unwrap() = 10;
    assert_eq!(store.status(root), Ok(Status::Interrupted));
    assert_eq!(store.execute(root, |value, _| Ok(*value)), Ok(10));
    assert_eq!(store.status(root), Ok(Status::Ready));
}
