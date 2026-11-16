# Solor

Solor is an experimental GUI framework written in Rust.

## Goals

- Fine-grained updates: limit state-driven updates, layout, and painting to
  affected parts of the interface.
- Low memory and CPU usage: reduce allocations during updates and avoid rendering
  frames when nothing changes.
- CPU and GPU rendering backends with consistent visual output.
- Custom drawing, shaders, visual effects, and complex animations.
- Consistent interface appearance across platforms, with platform integration for
  windows, text input, and accessibility.
- Composable components with explicit state and lifecycle management.
