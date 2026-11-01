# AI Agent Rules

**Pure Wayland only**: no X11 code, X libraries, X server or XWayland bridge in any app.

## Core Directives
1. **Never write code comments**: Zero inline, block, or trailing comments. No AI watermarks or commented-out code. Make code self-documenting through clear naming and types.
2. **Never add useless tests**: No tautological tests, mock-only verifications, or tests of language/compiler guarantees. Every test must test real domain logic and fail if that logic breaks.
3. **Reuse code first**: Search the app and the desktop crates before writing any helper, type, or utility. Never duplicate existing abstractions.
4. **Minimal surface area (YAGNI)**: Zero speculative code, unused builder patterns, or premature indirection. Solve problems with the fewest moving parts. Prefer standard library combinators and idiomatic composition over custom machinery.
5. **Purge code rot**: Delete unused imports, dead functions, unreferenced types, and stale tests in any file you touch. Leave code cleaner than you found it.

## Factory Verification
- **No panics**: No `.unwrap()`, `.expect()` or `panic!` in production code (each app denies them in `[lints.clippy]`). Use proper error handling.
- **Build first, test once**: each change only needs to compile: `cargo fmt` and `cargo check --locked --all-targets` in the app, under `flock ~/bande-linux/kbuild.lock ~/bande-linux/bin/gentle`, with `CARGO_TARGET_DIR` in your worktree. The full gate, `ci/gate.sh` (fmt, clippy `--all-targets -D warnings`, tests, all `--locked`, on a private session bus), runs once on `main` when a batch of work has merged, and `main` fixes whatever it finds then.

## Desktop crates
- The `bande-*` crates come from `bande-linux/desktop` as git dependencies, every app at the same `rev`. Change the pin only with `ci/bump-desktop.sh [REV]`, which re-locks and runs the gate on every app (`BUMP_NO_GATE=1` re-locks only). `ci/with-desktop.sh <desktop checkout> check --all-targets` tries a desktop branch on every app before it merges.
- A toolkit or library change lands in the desktop repo first; the apps pick it up with a bump. Never fork or vendor a desktop crate here.
- App shortcuts use Ctrl (`Ctrl+N`, `Shift+Ctrl+N`); Super is reserved for the system.

## Workflow
- Trunk-based: `main` always builds. Work on a short branch from `main`; merge when it compiles (fast-forward if possible, otherwise a merge commit). Never force-push.
- CI runs only for a release tag `<app>-vX.Y.Z` on `main` (see README). Never push tags or dispatch workflows unless the owner asks.
- Commit messages: one brief line saying what the commit does. No body, no trailers (no `Co-Authored-By`, no session links), nothing special.
