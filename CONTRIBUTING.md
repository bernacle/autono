# Contributing to Autono

Small, reviewable contributions are welcome. For major behavior or architecture
changes, open an issue describing the problem and proposed scope first.

Use Rust 1.89+ with a native C compiler. Build with `cargo build --workspace
--locked`. The repository commits `Cargo.lock` because it ships applications.
Before submitting a change, run:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
cargo doc --workspace --no-deps --locked
```

CI tests macOS and Linux. Process-group and shell acceptance tests use Unix;
portable state/storage tests do not require external coding CLIs. Tests use
temporary Autono homes and never alter your normal runtime data.

Keep domain behavior in `autono-core`, wire contracts in `autono-protocol`, HTTP
concerns in `autono-daemon`, and terminal presentation in `autono-cli`. Prefer
explicit ownership and small modules to general-purpose frameworks. Do not add
LLM SDKs or UI dependencies to the runtime foundation.

New persistent changes need a new migration; do not edit a migration that has
already shipped. A state transition and its lifecycle event must remain atomic.
Never infer controllable process ownership from a stored PID. Include meaningful
tests for changed behavior, especially crash and cancellation boundaries. Use
events or other explicit synchronization instead of sleeps to make tests ready.

Errors need context and must not convert failures into success. Avoid `unwrap()`
in runtime code. Document OS-specific limitations and any change to the local API
or security model. Describe the problem, final behavior, and validation in your
pull request. Contributions are provided under the project's MIT license.
