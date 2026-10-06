# GitHub Actions CI Workflow — Design

**Date**: 2026-07-13

## Overview

Add a GitHub Actions workflow that runs required CI checks on pull requests and pushes to `main`. Both jobs must pass before a PR can be merged.

## Workflow File

`.github/workflows/ci.yml`

## Triggers

- `pull_request` — all PRs
- `push` to `main` branch only

## Jobs

Two parallel jobs, both must succeed:

| Job | Command | Purpose |
|-----|---------|---------|
| `test` | `cargo test --workspace` | Run all 913+ tests across the workspace |
| `clippy` | `cargo clippy --all-targets --all-features` | Verify zero clippy warnings |

## Configuration

- **Runner**: `ubuntu-latest`
- **Rust toolchain**: `stable` via `dtolnay/rust-toolchain@stable`
- **Caching**: `Swatinem/rust-cache@v2` per job (keyed on `Cargo.lock` hash)
- **Formatting**: Not included — clippy style lints cover formatting concerns

## Activation

After merging the workflow, enable branch protection in GitHub repo settings:

1. Settings → Branches → Branch protection rules → Add rule
2. Branch pattern: `main`
3. Enable **Require status checks to pass before merging**
4. Select `test` and `clippy` as required checks
5. Enable **Require branches to be up to date before merging**

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| Two parallel jobs | Independent checks, faster feedback |
| No `cargo fmt` | Clippy style lints are sufficient, fmt is a local concern |
| `stable` channel | No nightly features needed, simplest approach |
| `Swatinem/rust-cache` | Standard, reliable caching keyed on Cargo.lock |
