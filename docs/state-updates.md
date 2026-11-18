# Explicit State Updates

## Scope and ownership

`Runtime<N>` owns a node tree, typed signals, and explicit bindings. A binding
connects one signal to one node through a callback. Multiple bindings may read
the same signal or update separate fields of the same node.

The runtime exposes an immutable tree view. Structural edits go through runtime
methods so subtree removal can release bindings before dropping node payloads.
Signals live until explicitly removed or the runtime is dropped. Copying or
dropping a signal or binding handle does not change ownership.

Each typed signal handle carries a `SignalId` containing the runtime's root
identity and a generational slot key. A scope can collect IDs from mixed
`Signal<T>` handles via `id()` and dispose them with `remove_signal_by_id`.
Reads, writes and bindings use typed handles. Stale and foreign
handles are rejected. Signal and binding generations have the same finite-wrap
limitation as the [node tree](node-tree.md).

## Scheduling contract

- Creating a binding queues its initial application; it does not run user code.
- `set` compares the current and new values using `PartialEq`. An equal value does
  not schedule work. Changed values are visible to `read` immediately.
- A changed signal queues only its subscribed bindings. Each binding appears at
  most once in the pending queue, including after multiple writes before a flush.
- `flush` executes pending bindings with the latest signal values and returns the
  number executed. Intermediate values are not delivered as events. Returning to
  the original value before flushing still leaves one scheduled application.
- A callback receives `&S` and `&mut N`, not runtime access. Signal writes,
  structural edits, and recursive flushing cannot be performed through those
  arguments. Automatic dependency collection and feedback cycles are outside scope.
- Callback order is unspecified. Bindings must not rely on one another's execution
  order. Deduplication is per binding, not per node.
- A callback panic propagates. Its task has been consumed, its payload changes are
  not rolled back, and unexecuted tasks remain queued. Retrying the callback is an
  explicit future state change, not an automatic retry loop.

## Lifecycle and costs

The runtime indexes bindings by both source signal and target node. Inverse vector
positions allow unbinding and removal from the pending queue without scanning
unrelated subscriptions. Removing a signal also removes its bindings. Removing a
node cancels its bindings before dropping its payload; subtree removal repeats
this for affected nodes only. Manual unbinding releases callback captures.

| Operation | Runtime work, excluding user code |
| --- | --- |
| `read` | O(1), including a checked type-erased lookup |
| `set` | Value comparison plus O(subscribers of the changed signal) |
| `bind` | Amortized O(1), with callback allocation and index growth |
| `unbind` | Expected O(1), including hash lookup and queue removal |
| `flush` | O(pending bindings); no tree or signal-table scan |
| `remove_signal` | Expected O(bindings for that signal) |
| `remove_subtree` | Tree deletion cost plus expected O(affected bindings) |

Signals and callbacks use boxed type erasure. Pending capacity is reserved when
bindings are registered, so scheduling and flushing do not need queue allocations.
User comparisons, values, callbacks, and destructors may still allocate. Storage
retains capacity at its high-water mark. Removing tasks removes queue entries
immediately rather than accumulating dead entries until a future flush.

## Acceptance and measurement

- With 10,000 independently bound nodes, one state change executes one callback
  and preserves every unrelated payload.
- Repeated writes merge into one application of the latest value.
- Shared signals update all subscribers; equality suppresses unnecessary work.
- Deleting queued nodes, unbinding, and removing signals release subscriptions
  and captures without affecting unrelated tasks or reused slots.
- Invalid handles and root removal leave the runtime unchanged.
- Callback and destructor panics preserve valid remaining scheduling indexes.
- Repeated registration and cancellation do not accumulate dead pending tasks.

Release measurements cover independent single updates, repeated writes, shared
signal fan-out, and bulk independent updates. Compare single-update work across
tree sizes. Report registration separately from measured update work; inspect
allocation behavior separately from elapsed-time measurement.

Run `cargo bench -p solor-core --bench state_updates --offline`. The harness warms
each workload and reports seven samples, including the median and range per batch.
Single-update batches contain one write and one flush. Burst batches contain 32
writes to one signal and one flush. Bulk batches write 10,000 independent signals;
fan-out batches write one signal with 10,000 subscribers. The equal-value case
measures suppressed work. Assertions check callback counts in every batch.
`begin`/`end` output markers delimit measurement regions for external allocation
profilers. Registration, warm-up, formatting of results, and teardown are excluded.

The workload uses scalar values and callbacks that copy those values into nodes.
Results describe this dispatch mechanism, not complete UI frames or application
memory. Allocation profiling must be reported separately from uninstrumented timing.

This milestone stops at explicit state-to-payload updates and lifecycle cleanup.
It does not include derived signals, multi-source bindings, automatic tracking,
layout/paint invalidation, rendering, event loops, or a complete reactive framework.
