use std::hint::black_box;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use solor_core::node_store::{AccessError, Context, NodeStore, Status};
use solor_core::tree::{NodeId, Tree};

const LIMIT: NonZeroUsize = NonZeroUsize::new(64).unwrap();
const SAMPLES: usize = 7;

trait Counter {
    fn increment(&mut self);
    fn value(&self) -> u64;
}

impl Counter for u64 {
    fn increment(&mut self) {
        *self += 1;
    }
    fn value(&self) -> u64 {
        *self
    }
}

fn flat_tree(count: usize) -> (Tree<Box<u64>>, Vec<NodeId>) {
    let mut tree = Tree::new(Box::new(0));
    let root = tree.root();
    let mut ids = Vec::with_capacity(count);
    ids.push(root);
    for _ in 1..count {
        ids.push(tree.append(root, Box::new(0)).unwrap());
    }
    (tree, ids)
}

fn flat_store(count: usize) -> (NodeStore<u64>, Vec<NodeId>) {
    let mut store = NodeStore::new(Box::new(0), LIMIT);
    let root = store.root();
    let mut ids = Vec::with_capacity(count);
    ids.push(root);
    for _ in 1..count {
        ids.push(store.append(root, Box::new(0)).unwrap());
    }
    (store, ids)
}

fn elapsed(iterations: usize, operation: &mut impl FnMut()) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        operation();
    }
    start.elapsed()
}

pub(super) fn paired(
    label: &str,
    iterations: usize,
    mut direct: impl FnMut(),
    mut loan: impl FnMut(),
) {
    for _ in 0..100 {
        direct();
        loan();
    }
    let mut a = [Duration::ZERO; SAMPLES];
    let mut b = a;
    println!("begin {label}");
    for sample in 0..SAMPLES {
        if sample % 2 == 0 {
            a[sample] = elapsed(iterations, &mut direct);
            b[sample] = elapsed(iterations, &mut loan);
        } else {
            b[sample] = elapsed(iterations, &mut loan);
            a[sample] = elapsed(iterations, &mut direct);
        }
    }
    println!("end {label}");
    a.sort_unstable();
    b.sort_unstable();
    let ns = |duration: Duration| duration.as_secs_f64() * 1e9 / iterations as f64;
    println!(
        "{label}: direct={:.1}ns [{:.1}, {:.1}], loan={:.1}ns [{:.1}, {:.1}] per batch; iterations={iterations}; samples={SAMPLES}",
        ns(a[3]),
        ns(a[0]),
        ns(a[6]),
        ns(b[3]),
        ns(b[0]),
        ns(b[6])
    );
}

fn visit(value: &mut u64, ctx: &mut Context<'_, u64>) -> Result<(), AccessError> {
    *value += 1;
    for index in 0..ctx.child_count() {
        ctx.execute_child(ctx.child(index).unwrap(), visit)?;
    }
    Ok(())
}

fn direct_visit(tree: &mut Tree<Box<u64>>, node: NodeId) {
    **tree.get_mut(node).unwrap() += 1;
    let count = tree.children(node).unwrap().len();
    for index in 0..count {
        let child = tree.children(node).unwrap().nth(index).unwrap();
        direct_visit(tree, child);
    }
}

pub fn run() {
    for (count, label) in [
        (100, "single/100"),
        (10_000, "single/10000"),
        (50_000, "single/50000"),
    ] {
        let (mut tree, a) = flat_tree(count);
        let (mut store, b) = flat_store(count);
        assert_eq!(tree.node_count(), store.node_count());
        let a_id = a[count / 2];
        let b_id = b[count / 2];
        paired(
            label,
            100_000,
            || {
                **black_box(&mut tree).get_mut(a_id).unwrap() += 1;
            },
            || {
                black_box(&mut store)
                    .execute(b_id, |value, _| {
                        *value += 1;
                        Ok(())
                    })
                    .unwrap();
            },
        );
        assert_eq!(**tree.get(a_id).unwrap(), *store.get(b_id).unwrap());
    }

    let mut erased_tree = Tree::<Box<dyn Counter>>::new(Box::new(0_u64));
    let mut erased_store = NodeStore::<dyn Counter>::new(Box::new(0_u64), LIMIT);
    let a = erased_tree.root();
    let b = erased_store.root();
    for _ in 1..10_000 {
        erased_tree.append(a, Box::new(0_u64)).unwrap();
        erased_store.append(b, Box::new(0_u64)).unwrap();
    }
    paired(
        "erased-single/10000",
        100_000,
        || black_box(black_box(&mut erased_tree).get_mut(a).unwrap()).increment(),
        || {
            black_box(&mut erased_store)
                .execute(b, |body, _| {
                    black_box(body).increment();
                    Ok(())
                })
                .unwrap();
        },
    );
    assert_eq!(
        erased_tree.get(a).unwrap().value(),
        erased_store.get(b).unwrap().value()
    );

    for (count, label) in [(2, "parent-child"), (10_001, "wide/10000-children")] {
        let (mut tree, a) = flat_tree(count);
        let (mut store, b) = flat_store(count);
        let iterations = if count == 2 { 100_000 } else { 100 };
        store
            .execute(b[0], |_, ctx| {
                assert_eq!(ctx.current(), b[0]);
                assert_eq!(ctx.parent(), None);
                assert_eq!(ctx.status(b[0]), Ok(Status::Running));
                Ok(())
            })
            .unwrap();
        paired(
            label,
            iterations,
            || direct_visit(black_box(&mut tree), a[0]),
            || {
                black_box(&mut store).execute(b[0], visit).unwrap();
            },
        );
        for (&a, &b) in a.iter().zip(&b) {
            assert_eq!(**tree.get(a).unwrap(), *store.get(b).unwrap());
            assert_eq!(store.status(b), Ok(Status::Ready));
        }
    }

    let mut tree = Tree::new(Box::new(0));
    let mut store = NodeStore::new(Box::new(0), LIMIT);
    let a = tree.root();
    let b = store.root();
    let mut a_leaf = a;
    let mut b_leaf = b;
    for _ in 1..32 {
        a_leaf = tree.append(a_leaf, Box::new(0)).unwrap();
        b_leaf = store.append(b_leaf, Box::new(0)).unwrap();
    }
    paired(
        "chain/32",
        10_000,
        || direct_visit(black_box(&mut tree), a),
        || {
            black_box(&mut store).execute(b, visit).unwrap();
        },
    );
    assert_eq!(**tree.get(a_leaf).unwrap(), *store.get(b_leaf).unwrap());

    paired(
        "direct-lookup/chain32",
        100_000,
        || {
            **black_box(&mut tree).get_mut(a_leaf).unwrap() += 1;
        },
        || {
            *black_box(&mut store).get_mut(b_leaf).unwrap() += 1;
        },
    );

    let a_child = tree.children(a).unwrap().next().unwrap();
    let b_child = store
        .execute(b, |_, ctx| Ok(ctx.child(0).unwrap()))
        .unwrap();
    assert_eq!(tree.remove_subtree(a_child), Ok(31));
    assert_eq!(store.remove_subtree(b_child), Ok(31));

    // Keep construction separate from timed execution for allocation/footprint probes.
    println!("begin storage/direct10000");
    let direct = black_box(flat_tree(10_000));
    println!("end storage/direct10000");
    drop(direct);
    println!("begin storage/loan10000");
    let loan = black_box(flat_store(10_000));
    println!("end storage/loan10000");
    drop(loan);
}
