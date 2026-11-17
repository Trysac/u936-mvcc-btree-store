# mvcc-btree-store

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/Trysac/u936-mvcc-btree-store/actions/workflows/ci.yml/badge.svg)](https://github.com/Trysac/u936-mvcc-btree-store/actions/workflows/ci.yml)

Embedded key-value store on a B+tree with MVCC snapshot isolation, first-committer-wins conflict detection and a CRC-checked write-ahead log that survives torn writes

## Problem

Describe in two or three sentences the concrete problem this utility solves, who hits it, and what
they do today without it. Replace this paragraph when the first real feature lands.

## Install

```sh
cargo add trysac-mvcc-btree-store --git https://github.com/Trysac/u936-mvcc-btree-store
```

Requires Rust 1.88 or newer. No dependencies outside the standard library.

## Usage

```rust
use mvcc_btree_store::{Error, normalize};

// normalize trims surrounding whitespace and lower-cases the text.
assert_eq!(normalize("  Hello World ").as_deref(), Ok("hello world"));

// Blank input is rejected explicitly instead of returning an empty string.
assert_eq!(mvcc_btree_store::normalize("   "), Err(Error::Blank));
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md) and [decisions](docs/decisions/)
- [Testing](docs/TESTING.md) and [security](docs/SECURITY.md)
- [Project record](docs/PROJECT.md) and [stack](docs/STACK.md)
- [Changelog](CHANGELOG.md) and [contributing guide](CONTRIBUTING.md)

## License

[MIT](LICENSE) (c) 2026 Miguel Gomez
