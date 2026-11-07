# AI Agent Rules

**Pure Wayland only**: no X11 code, X libraries, X server or XWayland bridge in any app.

## Core Directives
1. **Never write code comments**: Zero inline, block, or trailing comments. No AI watermarks or commented-out code. Make code self-documenting through clear naming and types.
2. **Few, purposeful tests only**: tests are end-to-end scenarios of what a user does and sees (real compositor headless, real test bus, real apps), or a regression test for a real bug or security boundary that fails without its fix. At most a couple per app or feature; the whole repo stays around 80–100 lines of tests per app (a guide, not a hard cap: most tests are not productive, so keep the count minimal). No unit tests of wiring or helpers (only genuinely tricky pure logic such as parsers or date maths), no tests that mirror the implementation, no mocks-only checks. **No screenshot/golden tests**: never add or regenerate image comparisons; assert behaviour (e.g. through the accessibility tree) instead.
3. **Reuse code first**: Search the app and the desktop crates before writing any helper, type, or utility. Never duplicate existing abstractions.
4. **Minimal surface area (YAGNI)**: Zero speculative code, unused builder patterns, or premature indirection. Solve problems with the fewest moving parts. Prefer standard library combinators and idiomatic composition over custom machinery.
5. **Purge code rot**: Delete unused imports, dead functions, unreferenced types, and stale tests in any file you touch. Leave code cleaner than you found it.

## Factory Verification
- **No panics**: No `.unwrap()`, `.expect()` or `panic!` in production code (the workspace `[workspace.lints.clippy]` denies them). Use proper error handling.
- **Build first, test once**: each change only needs to compile: `apps-check <worktree> -p bande-<app>` (`cargo fmt --all` and `cargo check --locked --all-targets` through the build queue, target in your worktree). Anything else heavy goes through the queue too: `build-run <worktree> cargo ...`, on whichever build machine is free (`build-run --here` to stay on the mini, for a benchmark or a binary you run next). The full gate, `ci/gate.sh` (fmt, clippy `--all-targets -D warnings`, tests, all `--locked`, on a private session bus), runs once on `main` when a batch of work has merged, and `main` fixes whatever it finds then.

## Desktop crates
- Every app is a `bande-app` app (`bande_app::run` in `src/main.rs`): windows, menus, standard keys, appearance, Settings… and single instance come from it, never from the app; `ci/check-app.sh` checks this. Start a new app with `ci/new-app.sh`.
- The repo is one Cargo workspace with one `Cargo.lock`; dependencies, lints and profiles live in the root `Cargo.toml` (`x.workspace = true` in the apps).
- The `bande-*` crates come from `bande-linux/desktop` as workspace git dependencies, all at the same `rev`. Change the pin only with `ci/bump-desktop.sh [REV]`, which re-locks and runs the gate on every app (`BUMP_NO_GATE=1` re-locks only). `ci/with-desktop.sh <desktop checkout> check --workspace --all-targets` tries a desktop branch on every app before it merges.
- A toolkit or library change lands in the desktop repo first; the apps pick it up with a bump. Never fork or vendor a desktop crate here.
- App shortcuts use Ctrl (`Ctrl+N`, `Shift+Ctrl+N`); Super is reserved for the system.

## Workflow
- Trunk-based: `main` always builds. Work on a short branch from `main`; merge when it compiles (fast-forward if possible, otherwise a merge commit). Never force-push.
- CI runs only for a release tag `<app>-vX.Y.Z` on `main` (see README). Never push tags or dispatch workflows unless the owner asks.
- Commit messages: one brief line saying what the commit does. No body, no trailers (no `Co-Authored-By`, no session links), nothing special.
