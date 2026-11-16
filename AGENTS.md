# Development Guidelines

## Working approach

- Honor the scope of the current request. An analysis request does not authorize
  implementation. A scaffolding request does not authorize functional systems.
- Before implementing a subsystem, state the concrete problem, scope, acceptance
  criteria, and excluded work. Complete one bounded milestone at a time.
- Validate architectural assumptions before building dependent systems.
- Do not expand the requested scope without an agreed need.
- Keep decisions easy to revise. Avoid speculative extension points,
  compatibility layers for uncommitted designs, and unused infrastructure.
- Use dependencies when they serve a concrete requirement. Explain substantial
  dependency additions; neither zero dependencies nor building everything from
  scratch is a goal by itself.

## Architecture and dependencies

- Keep crate boundaries aligned with concrete responsibilities.
- Keep the core independent of window systems, renderers, and platform APIs.
- Add crates only when an implemented responsibility needs a separate boundary.
  Do not create empty crates for a speculative architecture.
- Keep development rules in this file, project motivation and ideas in the README,
  and experiment goals and boundaries in `docs/experiment.md`. Put technical
  structure and usage documentation in separate files under `docs/` when needed.
- Update affected documentation when code or decisions make it inaccurate.

## Rust and documentation

- Use the workspace edition and shared lint configuration.
- Prefer clear ownership, explicit lifetimes of resources, and small APIs.
- Do not introduce unsafe code without a concrete need and an explicitly reviewed
  safety contract. The current workspace forbids it.
- Handle recoverable errors explicitly. Avoid panics for ordinary input failures.
- Use English for identifiers, comments, documentation, agent instructions, and
  default user interface text. Use Unix LF and standard rustfmt formatting.
- Document current behavior, limitations, and usage. Do not write documentation
  as a changelog or describe planned capabilities as implemented.
- Use `clap` if a command-line interface needs argument parsing.

## Correctness and performance

- Treat CPU cost, allocation behavior, and bounded memory as design constraints.
- Define lifecycle, identity, invalidation, and resource ownership invariants
  before introducing mechanisms that depend on them.
- Validate important invariants and concrete failure cases with focused tests.
  Do not add placeholder tests or tests that merely repeat implementation details.
- Measure relevant work, not just API invocation counts. Fewer render calls alone
  do not establish lower layout, paint, or frame cost.
- Use representative workloads and repeated comparable measurements. Record the
  workload, build profile, measurement boundary, and material uncertainty.
- Compare implementations with equivalent content and behavior. Keep benchmark
  observations separate from claims about complete applications.
- Compilation, unit tests, and offscreen output do not establish native window,
  input method, accessibility, visual, or cross-platform correctness.
- Do not claim an advantage over another implementation without a measured
  comparison.

## Validation and repository hygiene

- Run checks appropriate to the changed scope. Standard workspace checks are:
  - `cargo fmt --all -- --check`
  - `cargo check --workspace --all-targets --offline`
  - `cargo clippy --workspace --all-targets --offline -- -D warnings`
- Run focused tests when behavioral code exists. Broaden checks when the change
  affects additional crates or boundaries.
- Report what changed, what was verified, and the limits of that verification.
- Inspect repository status and relevant diffs before editing or committing when
  Git metadata is available. Do not initialize or repair Git metadata implicitly.
- Commit or push only when requested. Before committing, inspect staged paths and
  contents and exclude local configuration, credentials, absolute machine paths,
  logs, caches, build outputs, and local diagnostic artifacts.
- Do not launch applications, install components, or restart services unless the
  current task authorizes those actions.
