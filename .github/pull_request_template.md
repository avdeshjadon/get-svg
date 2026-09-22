## What does this PR do?

<!-- A one-paragraph description of the change. -->

## Checklist

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo test --all` passes (including new tests)
- [ ] No new public dependencies without a note in the PR body
- [ ] New untrusted-input handling routes through `src/security.rs`
- [ ] Default request rates / response limits are unchanged (or the change is opt-in)

## Related issues

<!-- Closes #123, refs #456, etc. -->

## Notes for reviewers

<!-- Anything unusual: how you validated it, mock-server behavior, etc. -->