use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use solor_core::state::{Runtime, StateError};
use solor_core::tree::TreeError;

#[derive(Default)]
struct Payload {
    value: usize,
    calls: usize,
}

fn apply(value: &usize, node: &mut Payload) {
    node.value = *value;
    node.calls += 1;
}

#[test]
fn one_of_ten_thousand_signals_updates_only_its_target() {
    let mut runtime = Runtime::new(Payload::default());
    let root = runtime.tree().root();
    let mut handles = Vec::new();
    for index in 0..10_000 {
        let node = if index == 0 {
            root
        } else {
            runtime.append(root, Payload::default()).unwrap()
        };
        let signal = runtime.create_signal(index);
        runtime.bind(signal, node, apply).unwrap();
        handles.push((signal, node));
    }
    assert_eq!(runtime.flush(), 10_000);
    assert!(runtime.set(handles[4_321].0, 20_000).unwrap());
    assert_eq!(runtime.pending_count(), 1);
    assert_eq!(runtime.flush(), 1);
    for (index, &(_, node)) in handles.iter().enumerate() {
        let payload = runtime.tree().get(node).unwrap();
        assert_eq!(payload.calls, if index == 4_321 { 2 } else { 1 });
        assert_eq!(payload.value, if index == 4_321 { 20_000 } else { index });
    }
    assert_eq!(runtime.flush(), 0);
}

#[test]
fn writes_coalesce_initial_application_reads_latest_and_equal_writes_do_nothing() {
    let mut runtime = Runtime::new(Payload::default());
    let root = runtime.tree().root();
    let signal = runtime.create_signal(10_usize);
    runtime.bind(signal, root, apply).unwrap();
    for value in 11..100 {
        runtime.set(signal, value).unwrap();
    }
    assert_eq!(runtime.read(signal), Ok(&99));
    assert_eq!(runtime.tree().get(root).unwrap().calls, 0);
    assert_eq!(runtime.pending_count(), 1);
    assert_eq!(runtime.flush(), 1);
    assert_eq!(runtime.tree().get(root).unwrap().value, 99);
    assert_eq!(runtime.set(signal, 99), Ok(false));
    assert_eq!(runtime.flush(), 0);
    runtime.set(signal, 100).unwrap();
    runtime.set(signal, 99).unwrap();
    assert_eq!(runtime.flush(), 1);
    assert_eq!(runtime.tree().get(root).unwrap().calls, 2);
}

#[test]
fn heterogeneous_signals_can_update_separate_fields_of_one_node() {
    let mut runtime = Runtime::new((String::new(), 0_u32));
    let root = runtime.tree().root();
    let text = runtime.create_signal(String::from("hello"));
    let number = runtime.create_signal(7_u32);
    runtime
        .bind(text, root, |value, node| node.0.clone_from(value))
        .unwrap();
    runtime
        .bind(number, root, |value, node| node.1 = *value)
        .unwrap();
    assert_eq!(runtime.flush(), 2);
    assert_eq!(runtime.tree().get(root), Some(&(String::from("hello"), 7)));
    runtime.set(text, String::from("world")).unwrap();
    assert_eq!(runtime.flush(), 1);
    assert_eq!(runtime.tree().get(root), Some(&(String::from("world"), 7)));
}

#[test]
fn cancellation_repairs_signal_node_and_pending_indexes() {
    let mut runtime = Runtime::new(Payload::default());
    let root = runtime.tree().root();
    let signals: Vec<_> = (0..4).map(|_| runtime.create_signal(1_usize)).collect();
    let nodes: Vec<_> = (0..24)
        .map(|_| runtime.append(root, Payload::default()).unwrap())
        .collect();
    let mut registrations = Vec::new();
    for (node_index, &node) in nodes.iter().enumerate() {
        for (source_index, &signal) in signals.iter().enumerate() {
            let binding = runtime
                .bind(signal, node, |value, payload| {
                    payload.value += value;
                    payload.calls += 1;
                })
                .unwrap();
            registrations.push((binding, node_index, source_index, true));
        }
    }
    for index in (0..registrations.len()).step_by(3) {
        runtime.unbind(registrations[index].0).unwrap();
        registrations[index].3 = false;
    }
    runtime.remove_signal(signals[1]).unwrap();
    for registration in &mut registrations {
        if registration.2 == 1 {
            registration.3 = false;
        }
    }
    runtime.remove_subtree(nodes[8]).unwrap();
    for registration in &mut registrations {
        if registration.1 == 8 {
            registration.3 = false;
        }
    }
    let live = registrations.iter().filter(|entry| entry.3).count();
    assert_eq!(runtime.binding_count(), live);
    assert_eq!(runtime.pending_count(), live);
    assert_eq!(runtime.flush(), live);
    runtime.set(signals[2], 5).unwrap();
    assert_eq!(
        runtime.flush(),
        registrations
            .iter()
            .filter(|entry| entry.3 && entry.2 == 2)
            .count()
    );
    for (index, &node) in nodes.iter().enumerate() {
        if index == 8 {
            continue;
        }
        let initial = registrations
            .iter()
            .filter(|entry| entry.3 && entry.1 == index)
            .count();
        let subsequent = registrations
            .iter()
            .filter(|entry| entry.3 && entry.1 == index && entry.2 == 2)
            .count();
        let payload = runtime.tree().get(node).unwrap();
        assert_eq!(payload.calls, initial + subsequent);
        assert_eq!(payload.value, initial + subsequent * 5);
    }
    for (binding, _, _, active) in registrations {
        assert_eq!(
            runtime.unbind(binding),
            if active {
                Ok(())
            } else {
                Err(StateError::InvalidBinding)
            }
        );
    }
    assert_eq!(runtime.binding_count(), 0);
    assert_eq!(runtime.pending_count(), 0);
}

#[test]
fn subtree_removal_releases_captures_and_cancels_queued_updates() {
    let mut runtime = Runtime::new(Payload::default());
    let root = runtime.tree().root();
    let branch = runtime.append(root, Payload::default()).unwrap();
    let child = runtime.append(branch, Payload::default()).unwrap();
    let signal = runtime.create_signal(1_usize);
    let capture = Rc::new(());
    let weak = Rc::downgrade(&capture);
    runtime
        .bind(signal, child, move |value, node| {
            let _keep = &capture;
            apply(value, node);
        })
        .unwrap();
    runtime.bind(signal, branch, apply).unwrap();
    runtime.bind(signal, root, apply).unwrap();
    assert_eq!(runtime.remove_subtree(branch), Ok(2));
    assert!(weak.upgrade().is_none());
    assert_eq!(runtime.binding_count(), 1);
    assert_eq!(runtime.pending_count(), 1);
    assert_eq!(runtime.signal_count(), 1);
    assert_eq!(runtime.flush(), 1);
    let replacement = runtime.append(root, Payload::default()).unwrap();
    runtime.set(signal, 2).unwrap();
    assert_eq!(runtime.flush(), 1);
    assert_eq!(runtime.tree().get(replacement).unwrap().calls, 0);
}

#[test]
fn stale_and_foreign_handles_and_root_removal_do_not_change_runtime() {
    let mut runtime = Runtime::new(Payload::default());
    let root = runtime.tree().root();
    let stale = runtime.create_signal(1_usize);
    let stale_binding = runtime.bind(stale, root, apply).unwrap();
    runtime.remove_signal(stale).unwrap();
    let live = runtime.create_signal(2_usize);
    runtime.bind(live, root, apply).unwrap();
    let mut other = Runtime::new(Payload::default());
    let foreign = other.create_signal(3_usize);
    let foreign_node = other.tree().root();
    let foreign_binding = other.bind(foreign, foreign_node, apply).unwrap();
    for invalid in [stale, foreign] {
        assert_eq!(runtime.read(invalid), Err(StateError::InvalidSignal));
        assert_eq!(runtime.set(invalid, 4), Err(StateError::InvalidSignal));
        assert_eq!(
            runtime.remove_signal(invalid),
            Err(StateError::InvalidSignal)
        );
        assert_eq!(
            runtime.bind(invalid, root, apply),
            Err(StateError::InvalidSignal)
        );
    }
    for invalid in [stale_binding, foreign_binding] {
        assert_eq!(runtime.unbind(invalid), Err(StateError::InvalidBinding));
    }
    assert_eq!(
        runtime.bind(live, foreign_node, apply),
        Err(StateError::Tree(TreeError::InvalidNode(foreign_node)))
    );
    assert_eq!(
        runtime.remove_subtree(foreign_node),
        Err(TreeError::InvalidNode(foreign_node))
    );
    assert_eq!(
        runtime.remove_subtree(root),
        Err(TreeError::CannotRemoveRoot)
    );
    assert_eq!(runtime.binding_count(), 1);
    assert_eq!(runtime.signal_count(), 1);
    assert_eq!(runtime.flush(), 1);
    assert_eq!(runtime.tree().get(root).unwrap().value, 2);
}

#[test]
fn registration_and_cancellation_without_flushing_do_not_accumulate_tasks() {
    let mut runtime = Runtime::new(Payload::default());
    let root = runtime.tree().root();
    let signal = runtime.create_signal(0_usize);
    for value in 1..2_000 {
        let node = runtime.append(root, Payload::default()).unwrap();
        let binding = runtime.bind(signal, node, apply).unwrap();
        runtime.set(signal, value).unwrap();
        runtime.remove_subtree(node).unwrap();
        assert_eq!(runtime.pending_count(), 0);
        assert_eq!(runtime.binding_count(), 0);
        assert_eq!(runtime.unbind(binding), Err(StateError::InvalidBinding));
    }
    runtime.bind(signal, root, apply).unwrap();
    assert_eq!(runtime.flush(), 1);
    assert_eq!(runtime.tree().get(root).unwrap().value, 1_999);
}

#[test]
fn callback_panic_consumes_one_task_and_preserves_remaining_tasks() {
    let mut runtime = Runtime::new(Payload::default());
    let root = runtime.tree().root();
    let signal = runtime.create_signal(1_usize);
    let first_call = Rc::new(Cell::new(true));
    for _ in 0..2 {
        let first_call = first_call.clone();
        runtime
            .bind(signal, root, move |value, node| {
                apply(value, node);
                assert!(!first_call.replace(false), "intentional callback panic");
            })
            .unwrap();
    }
    assert!(catch_unwind(AssertUnwindSafe(|| runtime.flush())).is_err());
    assert_eq!(runtime.pending_count(), 1);
    assert_eq!(runtime.flush(), 1);
    assert_eq!(runtime.tree().get(root).unwrap().calls, 2);
    runtime.set(signal, 2).unwrap();
    assert_eq!(runtime.flush(), 2);
}

struct PanicOnDrop;

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("intentional capture destructor panic");
    }
}

#[test]
fn capture_destructor_panic_preserves_node_and_remaining_indexes() {
    let mut runtime = Runtime::new(Payload::default());
    let root = runtime.tree().root();
    let child = runtime.append(root, Payload::default()).unwrap();
    let signal = runtime.create_signal(1_usize);
    runtime.bind(signal, child, apply).unwrap();
    let capture = PanicOnDrop;
    runtime
        .bind(signal, child, move |value, node| {
            let _keep = &capture;
            apply(value, node);
        })
        .unwrap();
    assert!(catch_unwind(AssertUnwindSafe(|| runtime.remove_subtree(child))).is_err());
    assert!(runtime.tree().get(child).is_some());
    assert_eq!(runtime.binding_count(), 1);
    assert_eq!(runtime.pending_count(), 1);
    runtime.set(signal, 2).unwrap();
    assert_eq!(runtime.flush(), 1);
    assert_eq!(runtime.remove_subtree(child), Ok(1));
    assert_eq!(runtime.binding_count(), 0);
}

#[test]
fn old_signal_value_destructor_panic_does_not_lose_notifications() {
    #[derive(PartialEq)]
    struct Value {
        number: usize,
        panic: bool,
    }
    impl Drop for Value {
        fn drop(&mut self) {
            assert!(!self.panic, "intentional value destructor panic");
        }
    }
    let mut runtime = Runtime::new(Payload::default());
    let root = runtime.tree().root();
    let signal = runtime.create_signal(Value {
        number: 1,
        panic: true,
    });
    runtime
        .bind(signal, root, |value, node| apply(&value.number, node))
        .unwrap();
    runtime.flush();
    assert!(
        catch_unwind(AssertUnwindSafe(|| runtime.set(
            signal,
            Value {
                number: 2,
                panic: false
            }
        )))
        .is_err()
    );
    assert_eq!(runtime.read(signal).unwrap().number, 2);
    assert_eq!(runtime.flush(), 1);
    assert_eq!(runtime.tree().get(root).unwrap().value, 2);
}
