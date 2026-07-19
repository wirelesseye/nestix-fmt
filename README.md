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
directories, and considers only `.rs` files. By default, it first passes each
complete Rust source file through the toolchain's `rustfmt` component, then also
uses `rustfmt` for ordinary Rust expressions embedded in the DSL.

`nestix-fmt` discovers `rustfmt.toml` and `.rustfmt.toml` using each source
file's directory, so files under nested configurations are formatted with their
nearest settings. Stdin uses the current working directory. Rustfmt settings
apply to the Rust source, while `max_width`, `tab_spaces`, and `hard_tabs` also
control wrapping and indentation inside `layout!`. Without a configuration,
the DSL defaults to four-space indentation and 100 columns. `--no-rustfmt`
does not read rustfmt configuration and always uses those defaults.
