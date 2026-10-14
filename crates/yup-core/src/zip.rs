//! A zip archive whose entries are stored rather than compressed.
//!
//! The packaging part of the standard (ECMA-376 Part 2) allows stored entries, and storing keeps
//! the archive simple enough to write by hand: the only computation is a CRC-32. Every entry
//! carries the same timestamp, the earliest a zip can hold, so the same workbook always gives
//! the same bytes.

/// The CRC-32 of `data`, with the polynomial zip uses.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc = CRC_TABLE[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

const CRC_TABLE: [u32; 256] = crc_table();

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// 1980-01-01, the earliest date a zip entry can carry, in the format's packed form.
const DATE: u16 = 0x0021;
/// Version 2.0 of the format, enough for stored entries in folders.
const VERSION: u16 = 20;

fn u16_le(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn size32(n: usize) -> u32 {
    u32::try_from(n).expect("a workbook is smaller than 4 GiB")
}

/// An archive of `files`, each a path and its contents, in the order given.
pub fn store(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in files {
        let offset = size32(out.len());
        let crc = crc32(data);
        let size = size32(data.len());
        let name_length = u16::try_from(name.len()).expect("a part name is short");

        // The local file header, then the data.
        u32_le(&mut out, 0x0403_4b50);
        u16_le(&mut out, VERSION);
        u16_le(&mut out, 0); // flags
        u16_le(&mut out, 0); // method: stored
        u16_le(&mut out, 0); // time
        u16_le(&mut out, DATE);
        u32_le(&mut out, crc);
        u32_le(&mut out, size); // compressed size
        u32_le(&mut out, size);
        u16_le(&mut out, name_length);
        u16_le(&mut out, 0); // extra field length
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        // The entry's record in the central directory.
        u32_le(&mut directory, 0x0201_4b50);
        u16_le(&mut directory, VERSION); // made by
        u16_le(&mut directory, VERSION); // needed to extract
        u16_le(&mut directory, 0); // flags
        u16_le(&mut directory, 0); // method: stored
        u16_le(&mut directory, 0); // time
        u16_le(&mut directory, DATE);
        u32_le(&mut directory, crc);
        u32_le(&mut directory, size);
        u32_le(&mut directory, size);
        u16_le(&mut directory, name_length);
        u16_le(&mut directory, 0); // extra field length
        u16_le(&mut directory, 0); // comment length
        u16_le(&mut directory, 0); // disk number
        u16_le(&mut directory, 0); // internal attributes
        u32_le(&mut directory, 0); // external attributes
        u32_le(&mut directory, offset);
        directory.extend_from_slice(name.as_bytes());
    }
    let directory_offset = size32(out.len());
    let directory_size = size32(directory.len());
    let count = u16::try_from(files.len()).expect("a workbook has fewer than 65,536 parts");
    out.extend_from_slice(&directory);

    // The end of the central directory.
    u32_le(&mut out, 0x0605_4b50);
    u16_le(&mut out, 0); // this disk
    u16_le(&mut out, 0); // the disk the directory starts on
    u16_le(&mut out, count);
    u16_le(&mut out, count);
    u32_le(&mut out, directory_size);
    u32_le(&mut out, directory_offset);
    u16_le(&mut out, 0); // comment length
    out
}

/// Reads back an archive that [`store`] wrote, checking each entry's CRC, for tests.
#[cfg(test)]
pub fn read(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
    let u16_at = |at: usize| usize::from(u16::from_le_bytes([archive[at], archive[at + 1]]));
    let u32_at = |at: usize| {
        u32::from_le_bytes([
            archive[at],
            archive[at + 1],
            archive[at + 2],
            archive[at + 3],
        ]) as usize
    };
    let end = archive.len() - 22;
    assert_eq!(
        u32_at(end),
        0x0605_4b50,
        "the archive ends with its directory"
    );
    let count = u16_at(end + 10);
    let mut at = u32_at(end + 16);
    let mut files = Vec::new();
    for _ in 0..count {
        assert_eq!(u32_at(at), 0x0201_4b50, "a directory record");
        let crc = u32_at(at + 16);
        let size = u32_at(at + 20);
        let name_length = u16_at(at + 28);
        let local = u32_at(at + 42);
        let name = String::from_utf8(archive[at + 46..at + 46 + name_length].to_vec())
            .expect("names are UTF-8");
        assert_eq!(u32_at(local), 0x0403_4b50, "a local header");
        let data_start = local + 30 + u16_at(local + 26) + u16_at(local + 28);
        let data = archive[data_start..data_start + size].to_vec();
        assert_eq!(crc32(&data) as usize, crc, "the CRC of {name}");
        files.push((name, data));
        at += 46 + name_length;
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crc_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn an_archive_reads_back() {
        let files = vec![
            ("a.txt".to_string(), b"hello".to_vec()),
            ("folder/b.xml".to_string(), b"<b/>".to_vec()),
        ];
        let archive = store(&files);
        assert!(archive.starts_with(b"PK\x03\x04"));
        assert_eq!(read(&archive), files);
    }
}
