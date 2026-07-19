# nestix-fmt

`nestix-fmt` formats the Nestix DSL inside Rust `layout!` macro invocations.
It leaves all Rust source outside those macro bodies unchanged.

```console
# Rewrite Rust files under a directory.
nestix-fmt src

# Check formatting without writing files.
nestix-fmt --check src

# Use it as a stdin/stdout filter.
nestix-fmt < src/main.rs > /tmp/main.rs
```

Directory traversal follows Git ignore rules, skips hidden entries and `target`
directories, and considers only `.rs` files. The formatter uses a fixed
four-space, 100-column style and does not require `rustfmt` at runtime.
