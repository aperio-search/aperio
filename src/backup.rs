use std::io::{Read, Write};

use crate::error::AppError;
use crate::store::Raw;

const MAGIC: &[u8; 8] = b"APIOEXPT";
const VERSION: u32 = 1;
const TABLE_COUNT: u32 = 4;

const TABLE_NAMES: [&str; 4] = ["meta", "queue", "docs", "inverted"];

/// Export all tables from the database into a portable binary format.
pub fn export_snapshot(env: &heed::Env) -> Result<Vec<u8>, AppError> {
    let txn = env.read_txn()?;
    let mut buf = Vec::new();

    buf.write_all(MAGIC)?;
    buf.write_all(&VERSION.to_le_bytes())?;
    buf.write_all(&TABLE_COUNT.to_le_bytes())?;

    for name in &TABLE_NAMES {
        let name_bytes = name.as_bytes();
        buf.write_all(&(name_bytes.len() as u16).to_le_bytes())?;
        buf.write_all(name_bytes)?;

        let kv_pairs: Vec<(Vec<u8>, Vec<u8>)> = match env.open_database::<Raw, Raw>(&txn, Some(name)) {
            Ok(Some(db)) => {
                let mut pairs = Vec::new();
                if let Ok(iter) = db.iter(&txn) {
                    for result in iter.flatten() {
                        pairs.push((result.0, result.1));
                    }
                }
                pairs
            }
            _ => Vec::new(),
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

fn clear_all_tables(env: &heed::Env) -> Result<(), AppError> {
    let mut wtxn = env.write_txn()?;
    for name in &TABLE_NAMES {
        if let Ok(db) = env.create_database::<Raw, Raw>(&mut wtxn, Some(name)) {
            db.clear(&mut wtxn)?;
        }
    }
    wtxn.commit()?;
    Ok(())
}

/// Import a previously exported binary snapshot into the database.
pub fn import_snapshot(env: &heed::Env, data: &[u8]) -> Result<(), AppError> {
    clear_all_tables(env)?;

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

    let mut wtxn = env.write_txn()?;

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

        let db = env.create_database::<Raw, Raw>(&mut wtxn, Some(&name))?;

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
            db.put(&mut wtxn, key.as_slice(), value.as_slice())?;
        }
    }

    wtxn.commit()?;

    Ok(())
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Internal(format!("backup I/O error: {e}"))
    }
}
