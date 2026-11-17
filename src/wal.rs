use crate::crc::crc32;
use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogOp {
    Put = 1,
    Delete = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub op: LogOp,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    pub ts: u64,
    pub entries: Vec<LogEntry>,
}

impl LogRecord {
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        let mut payload = Vec::new();
        payload.extend_from_slice(&self.ts.to_le_bytes());
        let count = u32::try_from(self.entries.len()).unwrap_or(u32::MAX);
        payload.extend_from_slice(&count.to_le_bytes());
        for entry in &self.entries {
            payload.push(entry.op as u8);
            let klen = u32::try_from(entry.key.len()).unwrap_or(u32::MAX);
            payload.extend_from_slice(&klen.to_le_bytes());
            payload.extend_from_slice(&entry.key);
            let vlen = u32::try_from(entry.value.len()).unwrap_or(u32::MAX);
            payload.extend_from_slice(&vlen.to_le_bytes());
            payload.extend_from_slice(&entry.value);
        }
        let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
        let checksum = crc32(&payload);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&checksum.to_le_bytes());
        out.extend_from_slice(&payload);
    }
}

#[derive(Debug)]
pub struct DecodedLog {
    pub records: Vec<LogRecord>,
    pub valid_bytes: usize,
    pub torn: bool,
}

/// Maximum payload size accepted when decoding a single WAL record (64 MiB).
const MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;

pub fn decode_log(bytes: &[u8]) -> Result<DecodedLog, Error> {
    let mut records = Vec::new();
    let mut offset = 0_usize;

    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        if remaining < 8 {
            return Ok(DecodedLog {
                records,
                valid_bytes: offset,
                torn: true,
            });
        }

        let len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let expected_crc = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);

        if len > MAX_RECORD_BYTES || remaining < 8 + len {
            return Ok(DecodedLog {
                records,
                valid_bytes: offset,
                torn: true,
            });
        }

        let payload = &bytes[offset + 8..offset + 8 + len];
        let next_offset = offset + 8 + len;
        let is_last = next_offset == bytes.len();

        let actual_crc = crc32(payload);
        if actual_crc != expected_crc {
            if is_last {
                return Ok(DecodedLog {
                    records,
                    valid_bytes: offset,
                    torn: true,
                });
            }
            return Err(Error::CorruptLog);
        }

        match decode_payload(payload) {
            Ok(record) => {
                records.push(record);
                offset = next_offset;
            }
            Err(err) => {
                if is_last {
                    return Ok(DecodedLog {
                        records,
                        valid_bytes: offset,
                        torn: true,
                    });
                }
                return Err(err);
            }
        }
    }

    Ok(DecodedLog {
        records,
        valid_bytes: offset,
        torn: false,
    })
}

fn decode_payload(payload: &[u8]) -> Result<LogRecord, Error> {
    if payload.len() < 12 {
        return Err(Error::CorruptLog);
    }
    let mut ts_bytes = [0_u8; 8];
    ts_bytes.copy_from_slice(&payload[0..8]);
    let ts = u64::from_le_bytes(ts_bytes);

    let count = u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]) as usize;
    let mut pos = 12_usize;
    let mut entries = Vec::with_capacity(count.min(1024));

    for _ in 0..count {
        if pos >= payload.len() {
            return Err(Error::CorruptLog);
        }
        let op = match payload[pos] {
            1 => LogOp::Put,
            2 => LogOp::Delete,
            _ => return Err(Error::CorruptLog),
        };
        pos += 1;

        if pos + 4 > payload.len() {
            return Err(Error::CorruptLog);
        }
        let klen = u32::from_le_bytes([
            payload[pos],
            payload[pos + 1],
            payload[pos + 2],
            payload[pos + 3],
        ]) as usize;
        pos += 4;
        if pos + klen > payload.len() {
            return Err(Error::CorruptLog);
        }
        let key = payload[pos..pos + klen].to_vec();
        pos += klen;

        if pos + 4 > payload.len() {
            return Err(Error::CorruptLog);
        }
        let vlen = u32::from_le_bytes([
            payload[pos],
            payload[pos + 1],
            payload[pos + 2],
            payload[pos + 3],
        ]) as usize;
        pos += 4;
        if pos + vlen > payload.len() {
            return Err(Error::CorruptLog);
        }
        let value = payload[pos..pos + vlen].to_vec();
        pos += vlen;

        entries.push(LogEntry { op, key, value });
    }

    if pos != payload.len() {
        return Err(Error::CorruptLog);
    }

    Ok(LogRecord { ts, entries })
}
