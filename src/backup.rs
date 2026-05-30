use std::io::{Read, Write};

use fjall::Readable;

use crate::error::AppError;

const MAGIC: &[u8; 8] = b"APIOEXPT";
const VERSION: u32 = 1;

/// Export all keyspaces from a database snapshot into a portable binary format.
///
/// Uses `fjall::Database::snapshot()` to obtain a point-in-time consistent view
/// without blocking concurrent writes.
pub fn export_snapshot(db: &fjall::Database) -> Result<Vec<u8>, AppError> {
    let snapshot = db.snapshot();
    let keyspace_names = db.list_keyspace_names();

    let mut buf = Vec::new();

    buf.write_all(MAGIC)?;
    buf.write_all(&VERSION.to_le_bytes())?;
    buf.write_all(&(keyspace_names.len() as u32).to_le_bytes())?;

    for name in &keyspace_names {
        let name_str: &str = name.as_ref();
        let keyspace = db.keyspace(name_str, fjall::KeyspaceCreateOptions::default)?;

        let name_bytes = name_str.as_bytes();
        buf.write_all(&(name_bytes.len() as u16).to_le_bytes())?;
        buf.write_all(name_bytes)?;

        let mut kv_pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for guard in snapshot.iter(&keyspace) {
            if let Ok((key, value)) = guard.into_inner() {
                kv_pairs.push((key.to_vec(), value.to_vec()));
            }
        }

        buf.write_all(&(kv_pairs.len() as u64).to_le_bytes())?;

        for (key, value) in &kv_pairs {
            buf.write_all(&(key.len() as u32).to_le_bytes())?;
            buf.write_all(key)?;
            buf.write_all(&(value.len() as u32).to_le_bytes())?;
            buf.write_all(value)?;
        }
    }

    Ok(buf)
}

/// Remove every key-value pair from every keyspace in the database.
fn clear_all_keyspaces(db: &fjall::Database) -> Result<(), AppError> {
    let names = db.list_keyspace_names();
    for name in &names {
        let name_str: &str = name.as_ref();
        let keyspace = db.keyspace(name_str, fjall::KeyspaceCreateOptions::default)?;
        let keys: Vec<Vec<u8>> = keyspace
            .iter()
            .filter_map(|g| g.into_inner().ok())
            .map(|(k, _)| k.to_vec())
            .collect();
        for key in &keys {
            keyspace.remove(key)?;
        }
    }
    Ok(())
}

/// Import a previously exported binary snapshot into the database.
///
/// Existing data is **erased first** — every key-value pair in every keyspace
/// is removed before inserting the archive contents. After import the
/// in-memory collection metadata cache is invalidated so the caller must
/// refresh it.
pub fn import_snapshot(db: &fjall::Database, data: &[u8]) -> Result<(), AppError> {
    clear_all_keyspaces(db)?;

    let mut reader = std::io::BufReader::new(data);

    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(AppError::BadRequest("invalid export format: bad magic".into()));
    }

    let mut version_buf = [0u8; 4];
    reader.read_exact(&mut version_buf)?;
    let version = u32::from_le_bytes(version_buf);
    if version != VERSION {
        return Err(AppError::BadRequest(format!(
            "unsupported export format version {version}, expected {VERSION}"
        )));
    }

    let mut count_buf = [0u8; 4];
    reader.read_exact(&mut count_buf)?;
    let keyspace_count = u32::from_le_bytes(count_buf);

    for _ in 0..keyspace_count {
        let mut len_buf = [0u8; 2];
        reader.read_exact(&mut len_buf)?;
        let name_len = u16::from_le_bytes(len_buf) as usize;

        let mut name_bytes = vec![0u8; name_len];
        reader.read_exact(&mut name_bytes)?;
        let name = String::from_utf8(name_bytes)
            .map_err(|_| AppError::BadRequest("invalid keyspace name in export".into()))?;

        let keyspace = db.keyspace(&name, fjall::KeyspaceCreateOptions::default)?;

        let mut kv_count_buf = [0u8; 8];
        reader.read_exact(&mut kv_count_buf)?;
        let kv_count = u64::from_le_bytes(kv_count_buf);

        for _ in 0..kv_count {
            let mut kl_buf = [0u8; 4];
            reader.read_exact(&mut kl_buf)?;
            let key_len = u32::from_le_bytes(kl_buf) as usize;

            let mut key = vec![0u8; key_len];
            reader.read_exact(&mut key)?;

            let mut vl_buf = [0u8; 4];
            reader.read_exact(&mut vl_buf)?;
            let val_len = u32::from_le_bytes(vl_buf) as usize;

            let mut value = vec![0u8; val_len];
            reader.read_exact(&mut value)?;

            keyspace.insert(&key, &value)?;
        }
    }

    Ok(())
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Internal(format!("backup I/O error: {e}"))
    }
}
