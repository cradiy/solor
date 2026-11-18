# Persistent Node Tree

## Scope

`solor_core::tree::Tree<T>` owns a single rooted tree with application-defined
payloads. Persistent means nodes remain in storage across edits; it does not mean
immutable snapshots or serialization. This module implements storage and
relationships, not UI construction, update propagation, layout, or painting.

## Identity and ownership

- Every tree has one root for its entire lifetime. The root cannot be removed.
- Every other node has exactly one parent. Children retain their declared sibling order.
- Inserting new children and removing subtrees can change topology. Moving
  nodes and attaching existing nodes are outside this milestone, so cycles cannot
  be introduced through the public API.
- `NodeId` is an opaque handle, not a pointer. It combines a SlotMap key with a
  process-local tree identifier. Handles from another tree are rejected.
- SlotMap preserves live keys when storage grows and changes their generation
  when slots are reused. Its finite generation counter can wrap after roughly
  2^31 reuse cycles of one slot; stale-handle rejection is subject to that limit.
  Handles are not suitable for durable storage or interchange between processes.
- The tree owns payloads. `get_mut` edits a payload without changing topology or
  identity. It does not track dependencies or schedule updates.

## Operations and costs

| Operation | Behavior | Cost |
| --- | --- | --- |
| `new` | Create a tree and its root payload | O(1) |
| `root`, `node_count` | Read root identity or live node count | O(1) |
| `get`, `get_mut` | Access a live payload, or return `None` | O(1) |
| `parent` | Return the parent, or `None` for the root | O(1) |
| `children` | Iterate immediate children in sibling order | O(child count) |
| `append` | Append a new child to a live parent | Amortized O(1) |
| `insert` | Insert a new child at an index from zero through the child count | Amortized O(1 + shifted siblings) |
| `remove_subtree` | Remove a node and its descendants; return the count | O(removed nodes + siblings of subtree root) |

Invalid structural operations return `TreeError` before modifying the tree.
Subtree deletion walks down the last child and back through parent links, removing
leaves first. It uses O(1) auxiliary storage and no recursion. The surviving
parent's child vector is compacted once, preserving sibling order. Each leaf is
unlinked before its payload is dropped, so a panicking payload destructor can
interrupt deletion but does not leave dangling topology links.

SlotMap and child vectors retain capacity after deletion for reuse. Memory follows
allocation high-water marks, not just live node count. Each node has a child vector;
empty leaf vectors do not allocate. This representation favors a small initial API
and contiguous child iteration. Frequent edits in very wide sibling lists need
measurement before a different representation is justified.

Tree identifiers are allocated from a checked process-wide 64-bit counter. Tree
creation panics on identifier exhaustion rather than reusing an identifier.

## Acceptance

- Live handles survive storage growth and unrelated edits.
- Slot reuse does not immediately make a removed handle valid again.
- Foreign handles cannot read, mutate, or remove nodes.
- Parent links, child order, and reachability remain consistent after edits.
- Removing a subtree releases its payloads exactly once and preserves siblings.
- Deep-chain removal and dropping a deep tree do not recurse through topology.
- Editing one payload in a 10,000-node tree preserves other payloads and handles.

These checks validate tree behavior only. They do not establish incremental UI
updates, rendering performance, or lower application memory consumption.
