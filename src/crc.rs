/// Computes the IEEE 802.3 CRC-32 checksum (reflected polynomial `0xEDB8_8320`).
///
/// # Examples
///
/// ```
/// use mvcc_btree_store::crc32;
/// assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
/// ```
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    const TABLE: [u32; 256] = build_table();
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in data {
        let idx = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = (crc >> 8) ^ TABLE[idx];
    }
    !crc
}

const fn build_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut i = 0_u32;
    while i < 256 {
        let mut crc = i;
        let mut bit = 0;
        while bit < 8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            bit += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
}
