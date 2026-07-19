# nestix-fmt

`nestix-fmt` formats the Nestix DSL inside Rust `layout!` macro invocations.

```console
# Rewrite Rust files under a directory.
nestix-fmt src

# Check formatting without writing files.
nestix-fmt --check src

# Format one package, a manifest's default package(s), or the whole workspace
# plus its local path dependencies (as with cargo fmt).
nestix-fmt --package nestix
nestix-fmt --manifest-path path/to/Cargo.toml
nestix-fmt --all

# Format the layout DSL without invoking rustfmt.
nestix-fmt --no-rustfmt src

# Use it as a stdin/stdout filter.
nestix-fmt < src/main.rs > /tmp/main.rs
```

Directory traversal follows Git ignore rules, skips hidden entries and `target`
directories, and considers only `.rs` files. The formatter uses a fixed
four-space, 100-column style. By default, it first passes each complete Rust
source file through the toolchain's `rustfmt` component, then also uses
`rustfmt` for ordinary Rust expressions embedded in the DSL.
