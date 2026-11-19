# Window frame acceptance

## Decision under review

For each window, the framework uses the **last frame accepted by its host** for
positional input. Acceptance means the host has taken responsibility for the
candidate's output in window submission order. For a synchronous Wayland host
this can happen in the dispatch turn that queues the surface commit. Physical
display and release of in-flight resources can occur later.

A window starts with no accepted frame and therefore no hit target. Its accepted
snapshot owns the ordered hit regions, mount identities, clips, scroll offset
and logical coordinate mapping corresponding to its drawing. Dispatch resolves
the frontmost hit there, then checks whether that mount is still live. A retired
hit consumes the lookup without falling through. A surviving mount that has
moved still receives hits at its accepted position until the next acceptance.
Removal immediately cancels its active press at the update boundary.

## Ownership during an update

| Boundary | Owner and effect |
| --- | --- |
| Apply input and structural changes | The component runtime drains pending work at an update boundary. Completed actions, binding deliveries and removals stand even if drawing later fails. |
| Complete layout and prepare drawing | The window's frame coordinator assembles one candidate from completed layout, drawing resources, and hit data for the same viewport. A layout publication can be reused across several candidates. |
| Accept output | On the host's acceptance of that exact candidate, the coordinator replaces the window's accepted snapshot as one operation before dispatching further input. It clears only the work covered by that candidate. |
| Release resources | Each old frame or in-flight submission keeps its own leases until its consumers release them. A buffer release can wake the host to submit pending work. |

A new edit or viewport change can supersede a candidate waiting to submit.
Discard an unsubmitted candidate when its inputs are obsolete and prepare the
latest state. If a submission is already irrevocably queued, record its
acceptance in order and leave newer work pending. The coordinator identifies
the actual candidate and its window configuration. Two attempts prepared
against the same accepted frame can share a layout snapshot or propose the
same next frame number.

## Retry ownership

- **Preparation fails.** The coordinator keeps the accepted frame and the
  unfinished frame request. Retry from current state, reusing valid layout and
  text caches. Only pending bindings run. Report a deterministic failure and
  wait for corrected input or an explicit retry to avoid spinning.
- **The host is waiting.** A pending configure, pacing callback or busy buffer
  leaves the request outstanding. The host signals readiness. The coordinator
  retries once eligible, coalescing intervening requests.
- **Upload or submission fails before acceptance.** For a recoverable failure
  the coordinator retains the pending request. The backend tracks the repairs
  still owed to the chosen buffer and the surface change still owed relative
  to the accepted output. A partially written buffer needs repair before use.
  Retry the candidate if current, or recompute against the accepted output
  for its successor. A terminal host or connection failure ends the window's
  submission path.
- **Pixels are unchanged.** The host can accept new hit data without a buffer
  upload when the candidate matches the accepted surface content and mapping
  and any required host state changes have been queued. A callback-only commit
  can still request pacing. An empty diff against the newest raster cache is
  insufficient if its preceding upload failed.

## Existing evidence and gap

The disclosure publishes its frame and reads title geometry from it as soon
as preparation succeeds. The Wayland host obtains that frame before updating
retained pixels and uploading a buffer. A frame callback clears the
pacing wait and a buffer release frees a slot. The buffer writer retains its
repair region on write error. This validates a buffer-local retry boundary.
The lifecycle fixture sets the accepted frame explicitly and checks retired
hit identities across drawing failure. The native host still propagates upload errors
out of its event loop. The headless test below exercises recovery and input
acceptance independently of Wayland.
[Disclosure layout](../crates/solor-render-cpu/examples/text_layout/disclosure_layout.rs),
[window submission](../crates/solor-linux/src/wayland/mod.rs),
[retained pixels](../crates/solor-render-cpu/src/damage.rs),
[buffer repair](../crates/solor-linux/src/wayland/buffer.rs)
and [lifecycle tests](../crates/solor-render-cpu/examples/text_layout/lifecycle_tests.rs).

## Headless acceptance evidence

The [test host](../crates/solor-render-cpu/examples/text_layout/lifecycle_tests/frame_commit.rs)
uses the existing state runtime, text layout and retained CPU rasterizer. It
keeps an accepted RGBA buffer and stages uploads separately. When the raster
changes, the host remembers the outstanding change across attempts. It compares
the candidate's size and bytes against the accepted buffer before copying a
whole buffer. This single-buffer comparison establishes correctness at unit
scale. Upload bandwidth, multiple shared buffers, native submission and
presentation timing remain outside this test.

- A binding moves a live text target 16 logical units right. The host writes
  half of the candidate's 5,600 bytes to staging, fails the upload and waits.
  The old buffer stays intact and only the old position hits. On readiness the
  host reuses the prepared candidate, uploads it and accepts its hit data.
  Its bytes match a fresh uncached raster. The move applied once and an idle
  wakeup produces no further acceptance.
- Resizing the viewport during the same wait discards the failed candidate.
  The old hit region remains active until readiness. The newly accepted layout
  uses the new viewport, clips hits at its shorter bottom edge and passes the
  same byte comparison. The resize delivers zero additional bindings.
- Removing a label and mounting an identical-looking replacement retires the
  old hit identity immediately. Once accepted, the replacement receives the
  hit with zero extra uploaded bytes. The existing host buffer matches a fresh
  raster of the new frame.

```sh
cargo test --offline --example text_window frame_commit
```
