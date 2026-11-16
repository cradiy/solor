use std::hint::black_box;
use std::time::{Duration, Instant};

use solor_core::state::{Runtime, Signal};

fn independent(count: usize) -> (Runtime<u64>, Vec<Signal<u64>>) {
    let mut runtime = Runtime::new(0);
    let root = runtime.tree().root();
    let mut signals = Vec::with_capacity(count);
    for index in 0..count {
        let node = if index == 0 {
            root
        } else {
            runtime.append(root, 0).unwrap()
        };
        let signal = runtime.create_signal(0_u64);
        runtime
            .bind(signal, node, |value, node| *node = *value)
            .unwrap();
        signals.push(signal);
    }
    assert_eq!(runtime.flush(), count);
    (runtime, signals)
}

fn measure(label: &str, iterations: usize, mut operation: impl FnMut()) {
    for _ in 0..100 {
        operation();
    }
    let mut samples = [Duration::ZERO; 7];
    // These markers delimit updates for external allocation profilers.
    println!("begin {label}");
    for sample in &mut samples {
        let start = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        *sample = start.elapsed();
    }
    println!("end {label}");
    samples.sort_unstable();
    println!(
        "{label}: median={:.1} ns/batch min={:.1} max={:.1}; iterations={iterations}, samples={}",
        samples[3].as_secs_f64() * 1e9 / iterations as f64,
        samples[0].as_secs_f64() * 1e9 / iterations as f64,
        samples[6].as_secs_f64() * 1e9 / iterations as f64,
        samples.len(),
    );
}

fn main() {
    for (count, label) in [
        (100, "single/100"),
        (10_000, "single/10000"),
        (50_000, "single/50000"),
    ] {
        let (mut runtime, signals) = independent(count);
        let signal = signals[count / 2];
        let mut value = 0_u64;
        measure(label, 100_000, || {
            let runtime = black_box(&mut runtime);
            value += 1;
            assert!(runtime.set(signal, black_box(value)).unwrap());
            assert_eq!(black_box(runtime.flush()), 1);
        });
    }

    let (mut runtime, signals) = independent(10_000);
    let mut value = 0_u64;
    measure("burst32/10000", 10_000, || {
        let runtime = black_box(&mut runtime);
        for _ in 0..32 {
            value += 1;
            assert!(runtime.set(signals[5_000], black_box(value)).unwrap());
        }
        assert_eq!(black_box(runtime.flush()), 1);
    });

    let (mut runtime, signals) = independent(10_000);
    let mut value = 0_u64;
    measure("bulk/10000", 100, || {
        let runtime = black_box(&mut runtime);
        value += 1;
        for &signal in &signals {
            assert!(runtime.set(signal, black_box(value)).unwrap());
        }
        assert_eq!(black_box(runtime.flush()), 10_000);
    });

    let mut runtime = Runtime::new(0_u64);
    let root = runtime.tree().root();
    let signal = runtime.create_signal(0_u64);
    for index in 0..10_000 {
        let node = if index == 0 {
            root
        } else {
            runtime.append(root, 0).unwrap()
        };
        runtime
            .bind(signal, node, |value, node| *node = *value)
            .unwrap();
    }
    assert_eq!(runtime.flush(), 10_000);
    let mut value = 0_u64;
    measure("fanout/10000", 100, || {
        let runtime = black_box(&mut runtime);
        value += 1;
        assert!(runtime.set(signal, black_box(value)).unwrap());
        assert_eq!(black_box(runtime.flush()), 10_000);
    });
    measure("equal/10000", 100_000, || {
        let runtime = black_box(&mut runtime);
        assert!(!runtime.set(signal, black_box(value)).unwrap());
        assert_eq!(black_box(runtime.flush()), 0);
    });
}
