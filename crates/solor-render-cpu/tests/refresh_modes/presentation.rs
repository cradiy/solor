//! End-to-end acceptance using compositor feedback from the native workload.
use anyhow::{Context as _, Result, ensure};
use std::{
    collections::BTreeMap,
    io::Read,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

fn native_trace(workload: &str) -> Result<String> {
    let mut child = Command::new(std::env::current_exe()?)
        .args([workload, "--exact", "--ignored", "--nocapture"])
        .env("SOLOR_TRACE_SUBMISSIONS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stderr = child.stderr.take().context("missing workload stderr")?;
    let (send, receive) = mpsc::channel();
    thread::scope(|scope| {
        scope.spawn(move || {
            let mut trace = String::new();
            let result = stderr.read_to_string(&mut trace).map(|_| trace);
            let _ = send.send(result);
        });
        // A compositor may suspend callbacks indefinitely for an obscured window.
        let trace = match receive.recv_timeout(Duration::from_secs(20)) {
            Ok(result) => result,
            Err(error) => {
                let _ = child.kill();
                child.wait()?;
                return Err(error).context("native window did not complete the frame sample");
            }
        };
        let status = child.wait()?;
        let trace = trace?;
        ensure!(status.success(), "native workload failed: {trace}");
        Ok(trace)
    })
}

fn check_native_rate(workload: &str) -> Result<()> {
    let trace = native_trace(workload)?;
    let mut feedback = BTreeMap::new();
    for line in trace.lines() {
        let (commit, time) = if let Some(rest) = line.strip_prefix("presented=") {
            let mut fields = rest.split_ascii_whitespace();
            let commit = fields.next().context("missing commit")?;
            let stamp = fields
                .next()
                .and_then(|s| s.strip_prefix("time="))
                .context("missing presentation time")?;
            let (sec, ns) = stamp.split_once('.').context("invalid presentation time")?;
            ensure!(ns.len() == 9, "invalid presentation nanoseconds");
            (commit, Some(Duration::new(sec.parse()?, ns.parse()?)))
        } else if let Some(commit) = line.strip_prefix("discarded=") {
            (commit, None)
        } else {
            continue;
        };
        ensure!(
            feedback.insert(commit.parse::<usize>()?, time).is_none(),
            "duplicate feedback for {commit}"
        );
    }

    // Bracket the measured interval with visible commits. The workload continues
    // for ten more frame opportunities so feedback can arrive asynchronously.
    let (&first, _) = feedback
        .range(..=21)
        .rev()
        .find(|(_, t)| t.is_some())
        .context("no initial presentation feedback")?;
    let (&last, _) = feedback
        .range(251..)
        .find(|(_, t)| t.is_some())
        .context("no final presentation feedback")?;
    let mut shown = Vec::new();
    for commit in first..=last {
        if let Some(time) = feedback
            .get(&commit)
            .with_context(|| format!("missing feedback for commit {commit}"))?
        {
            shown.push(*time);
        }
    }
    let mut gaps = Vec::with_capacity(shown.len() - 1);
    for pair in shown.windows(2) {
        ensure!(pair[1] > pair[0], "presentation clock did not advance");
        gaps.push(pair[1] - pair[0]);
    }
    let elapsed = shown[shown.len() - 1] - shown[0];
    let fps = gaps.len() as f64 / elapsed.as_secs_f64();
    gaps.sort_unstable();
    let p95 = gaps[(gaps.len() * 95).div_ceil(100) - 1];
    println!(
        "{workload} commits={first}..={last} presented={} discarded={} fps={fps:.2} interval_p95_ms={:.3}",
        shown.len(),
        last - first + 1 - shown.len(),
        p95.as_secs_f64() * 1000.0
    );
    ensure!(fps >= 120.0, "measured {fps:.2} fps, target 120 fps");
    Ok(())
}

#[test]
#[ignore = "requires a visible high-refresh Wayland output with presentation feedback"]
fn minimal_window_presents_at_120_fps() -> Result<()> {
    check_native_rate("wayland_minimal_frame_cadence")
}

#[test]
#[ignore = "requires a visible high-refresh Wayland output with presentation feedback"]
fn mixed_window_presents_at_120_fps() -> Result<()> {
    check_native_rate("wayland_mixed_frame_cadence")
}
