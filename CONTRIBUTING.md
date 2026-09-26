# Contributing

This project is licensed under the [MIT License](LICENSE). Copyright Daniel Gray.

**Sign off every commit** under the [Developer Certificate of Origin](https://developercertificate.org/) (DCO) v1.1:

```bash
git commit -s -m "your message"
```

That certifies you have the right to submit the change under MIT.

## Checks

```bash
cargo test --workspace --exclude shalt-app
cargo test -p shalt-core --offline
```

`examples/invoice/demo.sh` needs Python and a cucumber-family runner for the sample SUT.
`examples/rust-billing` is a cucumber-rs workspace.

Two rules that matter more than style:

1. **Test integrity through the public entry point.** A helper that is never called is not a guarantee.
2. **Absence of evidence is not green.** Pending, skipped, and unbound steps are not passes. A Then that acts instead of observing is a fake door — `shalt verify` must flag it.

Do not commit `~/.shalt/config.toml`, `.env`, keys, or credentials.
