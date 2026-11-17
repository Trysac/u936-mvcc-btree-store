use std::fmt;
use std::io;

/// Errors returned by [`crate::Store`] and [`crate::Txn`] operations.
///
/// Error formatting never includes user keys or values.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Another transaction committed a write to at least one overlapping key after this
    /// transaction's snapshot.
    WriteConflict,
    /// A key exceeded [`crate::Options::max_key_bytes`].
    KeyTooLarge,
    /// A value exceeded [`crate::Options::max_value_bytes`].
    ValueTooLarge,
    /// The number of active transactions reached [`crate::Options::max_active_txns`].
    TooManyTransactions,
    /// The write-ahead log failed structural or CRC validation prior to the final record.
    CorruptLog,
    /// An underlying storage I/O operation failed.
    Io(io::Error),
}

impl Error {
    /// Stable, machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::WriteConflict => "write_conflict",
            Self::KeyTooLarge => "key_too_large",
            Self::ValueTooLarge => "value_too_large",
            Self::TooManyTransactions => "too_many_transactions",
            Self::CorruptLog => "corrupt_log",
            Self::Io(_) => "io",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WriteConflict => f.write_str("write_conflict: concurrent modification detected"),
            Self::KeyTooLarge => f.write_str("key_too_large: key length exceeds configured limit"),
            Self::ValueTooLarge => {
                f.write_str("value_too_large: value length exceeds configured limit")
            }
            Self::TooManyTransactions => {
                f.write_str("too_many_transactions: active transaction limit reached")
            }
            Self::CorruptLog => {
                f.write_str("corrupt_log: write-ahead log checksum or format error")
            }
            Self::Io(_) => f.write_str("io: underlying storage operation failed"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
