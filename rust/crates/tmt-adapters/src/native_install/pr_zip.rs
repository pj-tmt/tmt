//! Actions ZIP transport reads fixed members in memory, never archive paths on disk.

use super::{invalid, pr_catalog::Member};
use std::{
    collections::BTreeMap,
    io::{self, Cursor, Read},
};

/// Bound the central-directory count before the decoder allocates metadata.
/// The ZIP reader coalesces duplicate names, so its returned length alone cannot
/// establish the declared inventory. ZIP64/multi-disk transports are unavailable
/// within this small fixed-member acquisition path.
fn inventory_count(bytes: &[u8], expected: usize) -> io::Result<()> {
    let bad = || invalid("Invalid or excessive Actions ZIP inventory.");
    let start = bytes.len().saturating_sub(22 + u16::MAX as usize);
    let end = bytes[start..]
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .map(|offset| start + offset)
        .ok_or_else(bad)?;
    let record = bytes.get(end..end + 22).ok_or_else(bad)?;
    let small = |offset| u16::from_le_bytes([record[offset], record[offset + 1]]);
    let large = |offset| {
        u32::from_le_bytes([
            record[offset],
            record[offset + 1],
            record[offset + 2],
            record[offset + 3],
        ])
    };
    if small(4) != 0
        || small(6) != 0
        || usize::from(small(8)) != expected
        || usize::from(small(10)) != expected
        || expected == 0
        || expected > 2
        || end.checked_add(22 + usize::from(small(20))) != Some(bytes.len())
        || usize::try_from(large(16))
            .ok()
            .and_then(|offset| offset.checked_add(large(12) as usize))
            != Some(end)
    {
        return Err(bad());
    }
    Ok(())
}

pub(super) fn members(
    bytes: &[u8],
    maximum: usize,
    expected: &[Member],
) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let bad = || invalid("Actions ZIP does not match its fixed member inventory.");
    if bytes.len() > maximum {
        return Err(bad());
    }
    inventory_count(bytes, expected.len())?;
    let mut archive = zip::ZipArchive::with_config(
        zip::read::Config {
            archive_offset: zip::read::ArchiveOffset::Known(0),
        },
        Cursor::new(bytes),
    )
    .map_err(|_| bad())?;
    if archive.len() != expected.len()
        || archive.offset() != 0
        || archive.has_overlapping_files().map_err(|_| bad())?
    {
        return Err(bad());
    }
    let mut result = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|_| bad())?;
        let member = expected
            .iter()
            .find(|member| member.name.as_bytes() == file.name_raw())
            .ok_or_else(bad)?;
        if !file.is_file()
            || file.encrypted()
            || file.size() != member.bytes
            || file.unix_mode().is_some_and(|mode| {
                let kind = mode & 0o170000;
                (kind != 0 && kind != 0o100000) || mode & 0o7000 != 0
            })
        {
            return Err(bad());
        }
        let mut contents = Vec::new();
        file.by_ref()
            .take(member.bytes.checked_add(1).ok_or_else(bad)?)
            .read_to_end(&mut contents)
            .map_err(|_| bad())?;
        if contents.len() as u64 != member.bytes
            || super::artifact::digest(&contents) != member.sha256
            || result.insert(member.name.clone(), contents).is_some()
        {
            return Err(bad());
        }
    }
    Ok(result)
}

/// The catalog member has no self-declared raw digest or size; its transport
/// API digest is checked by acquisition before this bounded fixed-name read.
pub(super) fn catalog(bytes: &[u8]) -> io::Result<Vec<u8>> {
    use super::pr_catalog::{CATALOG_LIMIT, CATALOG_ZIP_LIMIT};
    let bad = || invalid("Invalid Actions catalog ZIP.");
    if bytes.len() > CATALOG_ZIP_LIMIT {
        return Err(bad());
    }
    inventory_count(bytes, 1)?;
    let mut archive = zip::ZipArchive::with_config(
        zip::read::Config {
            archive_offset: zip::read::ArchiveOffset::Known(0),
        },
        Cursor::new(bytes),
    )
    .map_err(|_| bad())?;
    if archive.len() != 1 || archive.offset() != 0 {
        return Err(bad());
    }
    let mut file = archive.by_index(0).map_err(|_| bad())?;
    if file.name_raw() != b"catalog.json"
        || !file.is_file()
        || file.encrypted()
        || file.size() == 0
        || file.size() > CATALOG_LIMIT as u64
        || file.unix_mode().is_some_and(|mode| {
            let kind = mode & 0o170000;
            (kind != 0 && kind != 0o100000) || mode & 0o7000 != 0
        })
    {
        return Err(bad());
    }
    let size = file.size();
    let mut result = Vec::new();
    file.by_ref()
        .take(CATALOG_LIMIT as u64 + 1)
        .read_to_end(&mut result)
        .map_err(|_| bad())?;
    if result.len() as u64 != size {
        return Err(bad());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, contents) in files {
            writer
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
    fn member(name: &str, bytes: &[u8]) -> Member {
        Member {
            name: name.into(),
            bytes: bytes.len() as u64,
            sha256: super::super::artifact::digest(bytes),
        }
    }
    #[test]
    fn fixed_catalog_and_payload_members_are_returned_byte_exact() {
        assert_eq!(catalog(&zip(&[("catalog.json", b"{}")])).unwrap(), b"{}");
        let archive = zip(&[
            ("dist-manifest.json", b"manifest"),
            ("tmt-cli-target.tar.gz", b"archive"),
        ]);
        let expected = [
            member("dist-manifest.json", b"manifest"),
            member("tmt-cli-target.tar.gz", b"archive"),
        ];
        let result = members(&archive, archive.len(), &expected).unwrap();
        assert_eq!(result["dist-manifest.json"], b"manifest");
        assert_eq!(result["tmt-cli-target.tar.gz"], b"archive");
    }
    #[test]
    fn transport_refuses_paths_extras_digest_and_size_changes() {
        for name in [
            "../catalog.json",
            "/catalog.json",
            "nested/catalog.json",
            "catalog.json/",
        ] {
            assert!(catalog(&zip(&[(name, b"{}")])).is_err());
        }
        assert!(catalog(&zip(&[("catalog.json", b"{}"), ("extra", b"extra")])).is_err());
        let archive = zip(&[("one", b"one")]);
        for expected in [
            member("one", b"two"),
            member("one", b"longer"),
            member("other", b"one"),
        ] {
            assert!(members(&archive, archive.len(), &[expected]).is_err());
        }
        assert!(members(&archive, archive.len() - 1, &[member("one", b"one")]).is_err());
    }
    #[test]
    fn declared_count_fences_decoder_duplicate_name_coalescing_and_allocation() {
        let mut archive = zip(&[("one", b"one"), ("two", b"two")]);
        // Both local and central headers now contain the same raw member name.
        for index in 0..archive.len() - 2 {
            if &archive[index..index + 3] == b"two" {
                archive[index..index + 3].copy_from_slice(b"one");
            }
        }
        assert!(
            members(
                &archive,
                archive.len(),
                &[member("one", b"one"), member("two", b"two")]
            )
            .is_err()
        );
        let mut huge = zip(&[("catalog.json", b"{}")]);
        let end = huge.len() - 22;
        huge[end + 8..end + 12].copy_from_slice(&[255; 4]);
        assert!(catalog(&huge).is_err());
    }
    #[test]
    fn catalog_refuses_symlinks_special_entries_and_expansion_overflow() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_symlink(
                "catalog.json",
                "outside",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        assert!(catalog(&writer.finish().unwrap().into_inner()).is_err());
        let contents = vec![b'x'; super::super::pr_catalog::CATALOG_LIMIT + 1];
        assert!(catalog(&zip(&[("catalog.json", &contents)])).is_err());
    }
}
