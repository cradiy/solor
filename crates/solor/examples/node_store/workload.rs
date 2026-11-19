use std::hint::black_box;
use std::num::NonZeroUsize;

use solor::node_store::{AccessError, Context, NodeStore, Status};
use solor::tree::{NodeId, Tree};

use super::measure::paired;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Output {
    value: u64,
    visits: u64,
}

trait Body {
    fn begin(&mut self, input: u64) -> u64;
    fn finish(&mut self, result: u64) -> u64;
    fn output(&self) -> Output;
}

fn mix(value: u64, input: u64) -> u64 {
    value.rotate_left(7).wrapping_add(input) ^ 0x9e3779b97f4a7c15
}

struct Branch {
    salt: u64,
    output: Output,
}

impl Body for Branch {
    fn begin(&mut self, input: u64) -> u64 {
        mix(self.salt, input)
    }

    fn finish(&mut self, result: u64) -> u64 {
        self.output = Output {
            value: mix(result, self.salt),
            visits: self.output.visits + 1,
        };
        self.output.value
    }

    fn output(&self) -> Output {
        self.output
    }
}

struct Scalar {
    seed: u64,
    rounds: usize,
    output: Output,
}

impl Body for Scalar {
    fn begin(&mut self, input: u64) -> u64 {
        let mut result = input;
        for _ in 0..self.rounds {
            result = mix(result, self.seed);
        }
        result
    }

    fn finish(&mut self, result: u64) -> u64 {
        self.output.value = result;
        self.output.visits += 1;
        result
    }

    fn output(&self) -> Output {
        self.output
    }
}

struct Block {
    words: [u64; 16],
    rounds: usize,
    output: Output,
}

impl Body for Block {
    fn begin(&mut self, input: u64) -> u64 {
        let mut result = input;
        for _ in 0..self.rounds {
            for &word in &self.words {
                result = mix(result, word);
            }
        }
        result
    }

    fn finish(&mut self, result: u64) -> u64 {
        self.output.value = result;
        self.output.visits += 1;
        result
    }

    fn output(&self) -> Output {
        self.output
    }
}

fn branch(salt: u64) -> Box<dyn Body> {
    Box::new(Branch {
        salt,
        output: Output::default(),
    })
}

fn leaf(index: usize, rounds: usize) -> Box<dyn Body> {
    if index.is_multiple_of(2) {
        Box::new(Scalar {
            seed: index as u64,
            rounds,
            output: Output::default(),
        })
    } else {
        Box::new(Block {
            words: std::array::from_fn(|word| mix(word as u64, index as u64)),
            rounds,
            output: Output::default(),
        })
    }
}

fn execute(
    body: &mut dyn Body,
    ctx: &mut Context<'_, dyn Body>,
    input: u64,
) -> Result<u64, AccessError> {
    let mut result = body.begin(input);
    for index in 0..ctx.child_count() {
        let child = ctx.execute_child(ctx.child(index).unwrap(), |body, ctx| {
            execute(body, ctx, mix(input, index as u64))
        })?;
        result = mix(result, child);
    }
    Ok(body.finish(result))
}

fn direct_execute(tree: &mut Tree<Box<dyn Body>>, node: NodeId, input: u64) -> u64 {
    // Split the parent operation so no payload borrow spans a child invocation.
    let mut result = tree.get_mut(node).unwrap().begin(input);
    let count = tree.children(node).unwrap().len();
    for index in 0..count {
        let child = tree.children(node).unwrap().nth(index).unwrap();
        result = mix(
            result,
            direct_execute(tree, child, mix(input, index as u64)),
        );
    }
    tree.get_mut(node).unwrap().finish(result)
}

struct Fixture {
    direct: Tree<Box<dyn Body>>,
    loan: NodeStore<dyn Body>,
    nodes: Vec<(NodeId, NodeId)>,
    branches: Vec<(NodeId, NodeId)>,
}

impl Fixture {
    fn new(branches: usize, leaves: usize, rounds: usize, reused: bool) -> Self {
        let direct = Tree::new(branch(0));
        let loan = NodeStore::new(branch(0), NonZeroUsize::new(8).unwrap());
        let roots = (direct.root(), loan.root());
        let mut fixture = Self {
            direct,
            loan,
            nodes: vec![roots],
            branches: Vec::new(),
        };
        for group in 0..branches {
            let parents = (
                fixture
                    .direct
                    .append(roots.0, branch(group as u64))
                    .unwrap(),
                fixture.loan.append(roots.1, branch(group as u64)).unwrap(),
            );
            fixture.nodes.push(parents);
            fixture.branches.push(parents);
            let mut replacements = Vec::new();
            for item in 0..leaves {
                let index = group * leaves + item;
                let ids = (
                    fixture
                        .direct
                        .append(parents.0, leaf(index, rounds))
                        .unwrap(),
                    fixture.loan.append(parents.1, leaf(index, rounds)).unwrap(),
                );
                fixture.nodes.push(ids);
                if reused && item % 4 < 2 {
                    replacements.push((fixture.nodes.len() - 1, index));
                }
            }
            for &(position, _) in &replacements {
                let (a, b) = fixture.nodes[position];
                fixture.direct.remove_subtree(a).unwrap();
                fixture.loan.remove_subtree(b).unwrap();
            }
            for &(position, index) in replacements.iter().rev() {
                let stale = fixture.nodes[position];
                fixture.nodes[position] = (
                    fixture
                        .direct
                        .append(parents.0, leaf(index, rounds))
                        .unwrap(),
                    fixture.loan.append(parents.1, leaf(index, rounds)).unwrap(),
                );
                assert!(fixture.direct.get(stale.0).is_none());
                assert!(fixture.loan.get(stale.1).is_err());
            }
        }
        fixture
    }

    fn verify(&self) -> u64 {
        assert_eq!(self.direct.node_count(), self.loan.node_count());
        let mut visits = 0;
        for &(a, b) in &self.nodes {
            let direct = self.direct.get(a).unwrap().output();
            assert_eq!(direct, self.loan.get(b).unwrap().output());
            assert_eq!(self.loan.status(b), Ok(Status::Ready));
            visits += direct.visits;
        }
        visits
    }
}

pub fn run() {
    for reused in [false, true] {
        for (rounds, work) in [(1, "light"), (8, "compute")] {
            let mut fixture = Fixture::new(100, 100, rounds, reused);
            let storage = if reused { "reused" } else { "fresh" };
            for selected in [1, 10, 100] {
                // Independent subtree jobs supplied explicitly, not a dirty scheduler.
                let jobs: Vec<_> = if selected == 100 {
                    vec![fixture.nodes[0]]
                } else {
                    (0..selected)
                        .map(|index| fixture.branches[index * 100 / selected])
                        .collect()
                };
                let touched = if selected == 100 {
                    10_101
                } else {
                    selected * 101
                };
                let iterations = 2_000_000 / touched;
                let label = format!("mixed/{storage}/{work}/{selected}-branches");
                let before = fixture.verify();
                let mut direct_input = 0_u64;
                let mut loan_input = 0_u64;
                paired(
                    &label,
                    iterations,
                    || {
                        direct_input += 1;
                        for &(a, _) in &jobs {
                            black_box(direct_execute(
                                black_box(&mut fixture.direct),
                                a,
                                direct_input,
                            ));
                        }
                    },
                    || {
                        loan_input += 1;
                        for &(_, b) in &jobs {
                            black_box(
                                black_box(&mut fixture.loan)
                                    .execute(b, |body, ctx| execute(body, ctx, loan_input))
                                    .unwrap(),
                            );
                        }
                    },
                );
                assert_eq!(direct_input, loan_input);
                assert_eq!(fixture.verify() - before, direct_input * touched as u64);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_nested_execution_agree_and_subtree_jobs_preserve_other_outputs() {
        for reused in [false, true] {
            let mut fixture = Fixture::new(3, 5, 2, reused);
            let (a, b) = fixture.nodes[0];
            let expected = direct_execute(&mut fixture.direct, a, 42);
            assert_eq!(
                fixture.loan.execute(b, |body, ctx| execute(body, ctx, 42)),
                Ok(expected)
            );
            assert_eq!(fixture.verify(), 19);
            let before: Vec<_> = fixture
                .nodes
                .iter()
                .map(|&(a, _)| fixture.direct.get(a).unwrap().output())
                .collect();
            let (a, b) = fixture.branches[1];
            let expected = direct_execute(&mut fixture.direct, a, 17);
            assert_eq!(
                fixture.loan.execute(b, |body, ctx| execute(body, ctx, 17)),
                Ok(expected)
            );
            assert_eq!(fixture.verify(), 25);
            for (&(node, _), previous) in fixture.nodes.iter().zip(before) {
                if node != a && fixture.direct.parent(node).unwrap() != Some(a) {
                    assert_eq!(fixture.direct.get(node).unwrap().output(), previous);
                }
            }
        }
    }
}
