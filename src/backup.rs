use std::io::{Read, Write};

use redb::{ReadableDatabase, ReadableTable, TableDefinition};

use crate::error::AppError;
use crate::store::{DOCS, INVERTED, META, QUEUE};

const MAGIC: &[u8; 8] = b"APIOEXPT";
const VERSION: u32 = 1;
const TABLE_COUNT: u32 = 4;

/// Export all tables from the database into a portable binary format.
///
/// Uses `db.begin_read()` to obtain a point-in-time consistent view
/// without blocking concurrent writes.
pub fn export_snapshot(db: &redb::Database) -> Result<Vec<u8>, AppError> {
    let txn = db.begin_read()?;
    let mut buf = Vec::new();

    buf.write_all(MAGIC)?;
    buf.write_all(&VERSION.to_le_bytes())?;
    buf.write_all(&TABLE_COUNT.to_le_bytes())?;

    let tables: [(&str, TableDefinition<&[u8], &[u8]>); 3] = [
        ("queue", QUEUE),
        ("docs", DOCS),
        ("inverted", INVERTED),
    ];

    // Export META separately (uses &str keys)
    let name = "meta";
    let name_bytes = name.as_bytes();
    buf.write_all(&(name_bytes.len() as u16).to_le_bytes())?;
    buf.write_all(name_bytes)?;
    let kv_pairs: Vec<(Vec<u8>, Vec<u8>)> = match txn.open_table(META) {
        Ok(table) => {
            let mut pairs = Vec::new();
            if let Ok(iter) = table.iter() {
                for (key, value) in iter.flatten() {
                    pairs.push((key.value().as_bytes().to_vec(), value.value().to_vec()));
                }
            }
            pairs
        }
        Err(_) => Vec::new(),
    };
    buf.write_all(&(kv_pairs.len() as u64).to_le_bytes())?;
    for (key, value) in &kv_pairs {
        buf.write_all(&(key.len() as u32).to_le_bytes())?;
        buf.write_all(key)?;
        buf.write_all(&(value.len() as u32).to_le_bytes())?;
        buf.write_all(value)?;
    }

    for (name, table_def) in tables {
        let name_bytes = name.as_bytes();
        buf.write_all(&(name_bytes.len() as u16).to_le_bytes())?;
        buf.write_all(name_bytes)?;

        let kv_pairs: Vec<(Vec<u8>, Vec<u8>)> = match txn.open_table(table_def) {
            Ok(table) => {
                let mut pairs = Vec::new();
                if let Ok(iter) = table.iter() {
                    for (key, value) in iter.flatten() {
                        pairs.push((key.value().to_vec(), value.value().to_vec()));
                    }
                }
                pairs
            }
            Err(_) => Vec::new(),
        };

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

/// Remove every key-value pair from every table in the database.
fn clear_all_tables(db: &redb::Database) -> Result<(), AppError> {
    let txn = db.begin_write()?;
    {
        // META uses &str keys
        if let Ok(table) = txn.open_table(META) {
            let keys: Vec<String> = table
                .iter()?
                .flatten()
                .map(|(k, _)| k.value().to_string())
                .collect();
            if let Ok(mut table) = txn.open_table(META) {
                for key in &keys {
                    table.remove(key.as_str())?;
                }
            }
        }
        for table_def in [QUEUE, DOCS, INVERTED] {
            if let Ok(mut table) = txn.open_table(table_def) {
                let keys: Vec<Vec<u8>> = table
                    .iter()?
                    .flatten()
                    .map(|(k, _)| k.value().to_vec())
                    .collect();
                for key in &keys {
                    table.remove(key.as_slice())?;
                }
            }
        }
    }
    txn.commit()?;
    Ok(())
}

/// Import a previously exported binary snapshot into the database.
///
/// Existing data is **erased first** — every key-value pair in every table
/// is removed before inserting the archive contents. After import the
/// in-memory collection metadata cache is invalidated so the caller must
/// refresh it.
pub fn import_snapshot(db: &redb::Database, data: &[u8]) -> Result<(), AppError> {
    clear_all_tables(db)?;

    let mut reader = std::io::BufReader::new(data);

    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(AppError::BadRequest(
            "invalid export format: bad magic".into(),
        ));
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
    let _table_count = u32::from_le_bytes(count_buf);

    let txn = db.begin_write()?;
    {
        for _ in 0..TABLE_COUNT {
            let mut len_buf = [0u8; 2];
            reader.read_exact(&mut len_buf)?;
            let name_len = u16::from_le_bytes(len_buf) as usize;

            let mut name_bytes = vec![0u8; name_len];
            reader.read_exact(&mut name_bytes)?;
            let name = String::from_utf8(name_bytes)
                .map_err(|_| AppError::BadRequest("invalid table name in export".into()))?;

            let mut kv_count_buf = [0u8; 8];
            reader.read_exact(&mut kv_count_buf)?;
            let kv_count = u64::from_le_bytes(kv_count_buf);

            match name.as_str() {
                "meta" => {
                    let mut table = txn.open_table(META)?;
                    for _ in 0..kv_count {
                        let mut kl_buf = [0u8; 4];
                        reader.read_exact(&mut kl_buf)?;
                        let key_len = u32::from_le_bytes(kl_buf) as usize;
                        let mut key = vec![0u8; key_len];
                        reader.read_exact(&mut key)?;
                        let key_str = String::from_utf8(key)
                            .map_err(|_| AppError::BadRequest("invalid meta key".into()))?;
                        let mut vl_buf = [0u8; 4];
                        reader.read_exact(&mut vl_buf)?;
                        let val_len = u32::from_le_bytes(vl_buf) as usize;
                        let mut value = vec![0u8; val_len];
                        reader.read_exact(&mut value)?;
                        table.insert(key_str.as_str(), value.as_slice())?;
                    }
                }
                "queue" | "docs" | "inverted" => {
                    let def: TableDefinition<&[u8], &[u8]> = match name.as_str() {
                        "queue" => QUEUE,
                        "docs" => DOCS,
                        "inverted" => INVERTED,
                        _ => unreachable!(),
                    };
                    let mut table = txn.open_table(def)?;
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
                        table.insert(key.as_slice(), value.as_slice())?;
                    }
                }
                _ => {
                    return Err(AppError::BadRequest(format!("unknown table: {name}")));
                }
            }
        }
    }
    txn.commit()?;

    Ok(())
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Internal(format!("backup I/O error: {e}"))
    }
}