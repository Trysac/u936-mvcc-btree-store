# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added MVCC B+tree store (`Store`, `Txn`, `Storage`, `FileStorage`, `MemStorage`, `Options`, `Stats`, `Error`, `crc32`, and CLI runner) with snapshot isolation, first-committer-wins conflict detection, WAL crash recovery, and checkpointing.
- Initial project structure from the factory utility template.
