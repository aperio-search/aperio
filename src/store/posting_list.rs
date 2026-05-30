use roaring::RoaringTreemap;

use crate::error::AppError;

use super::SHARD_DELIM;
use super::config::PostingShard;
use super::roaring_from_slice;
use super::roaring_to_vec;

pub fn shard_key(word: &str, shard: usize) -> Vec<u8> {
    format!("{}{}{:04}", word, SHARD_DELIM, shard).into_bytes()
}

pub fn load_posting_shard(
    inverted: &fjall::Keyspace,
    word: &str,
    shard: usize,
) -> Result<Option<PostingShard>, AppError> {
    let key = shard_key(word, shard);
    match inverted.get(&key)? {
        Some(data) => {
            let shard: PostingShard =
                decode_rkyv!(PostingShard, &data).unwrap_or_else(|_| PostingShard {
                    first: String::new(),
                    last: String::new(),
                    ids: Vec::new(),
                });
            Ok(Some(shard))
        }
        None => Ok(None),
    }
}

pub fn list_shard_indices(inverted: &fjall::Keyspace, word: &str) -> Result<Vec<usize>, AppError> {
    let prefix = format!("{}{}", word, SHARD_DELIM).into_bytes();
    let mut indices: Vec<usize> = Vec::new();
    for guard in inverted.prefix(&prefix) {
        let (key, _) = guard.into_inner()?;
        if let Some(null_pos) = key.iter().rposition(|&b| b == SHARD_DELIM as u8) {
            let digit_bytes = &key[null_pos + 1..];
            let mut idx = 0;
            for &b in digit_bytes {
                idx = idx * 10 + (b - b'0') as usize;
            }
            indices.push(idx);
        }
    }
    indices.sort_unstable();
    Ok(indices)
}

pub fn find_shard_for_id(
    inverted: &fjall::Keyspace,
    word: &str,
    id: &str,
    indices: &[usize],
) -> Result<usize, AppError> {
    let first_shard =
        load_posting_shard(inverted, word, indices[0])?.unwrap_or_else(|| PostingShard {
            first: String::new(),
            last: String::new(),
            ids: Vec::new(),
        });

    if *id < *first_shard.first {
        return Ok(indices[0]);
    }

    let mut lo = 0usize;
    let mut hi = indices.len().saturating_sub(1);
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let shard = match load_posting_shard(inverted, word, indices[mid])? {
            Some(s) => s,
            None => {
                lo = mid + 1;
                continue;
            }
        };
        if *id < *shard.first {
            if mid == 0 {
                return Ok(indices[0]);
            }
            hi = mid - 1;
        } else if *id > *shard.last {
            lo = mid + 1;
        } else {
            return Ok(indices[mid]);
        }
    }
    Ok(indices[lo.min(indices.len().saturating_sub(1))])
}

pub fn add_to_posting_list(
    batch: &mut fjall::OwnedWriteBatch,
    inverted: &fjall::Keyspace,
    word: &str,
    id: &str,
    max_shard_size: usize,
) -> Result<(), AppError> {
    let marker_key = word.as_bytes();
    if inverted.get(marker_key)?.is_none() {
        batch.insert(inverted, marker_key, []);
    }

    let indices = list_shard_indices(inverted, word)?;

    if indices.is_empty() {
        let shard = PostingShard {
            first: id.to_string(),
            last: id.to_string(),
            ids: vec![id.to_string()],
        };
        let value = encode_rkyv!(&shard)?;
        batch.insert(inverted, shard_key(word, 0), &value);
        return Ok(());
    }

    let last_idx = *indices.last().unwrap();
    let last_shard =
        load_posting_shard(inverted, word, last_idx)?.unwrap_or_else(|| PostingShard {
            first: String::new(),
            last: String::new(),
            ids: Vec::new(),
        });

    if *id > *last_shard.last {
        if last_shard.ids.len() < max_shard_size {
            if last_shard.ids.binary_search(&id.to_string()).is_ok() {
                return Ok(());
            }
            let mut new_shard = last_shard;
            new_shard.ids.push(id.to_string());
            new_shard.last = id.to_string();
            let new_value = encode_rkyv!(&new_shard)?;
            batch.insert(inverted, shard_key(word, last_idx), &new_value);
        } else {
            let shard = PostingShard {
                first: id.to_string(),
                last: id.to_string(),
                ids: vec![id.to_string()],
            };
            let value = encode_rkyv!(&shard)?;
            batch.insert(inverted, shard_key(word, last_idx + 1), &value);
        }
        return Ok(());
    }

    let target = find_shard_for_id(inverted, word, id, &indices)?;
    let current = load_posting_shard(inverted, word, target)?.unwrap_or_else(|| PostingShard {
        first: String::new(),
        last: String::new(),
        ids: Vec::new(),
    });

    if current.ids.binary_search(&id.to_string()).is_ok() {
        return Ok(());
    }

    let pos = current.ids.binary_search(&id.to_string()).unwrap_err();
    let mut new_shard = current;
    new_shard.ids.insert(pos, id.to_string());
    if pos == 0 {
        new_shard.first = id.to_string();
    }
    if pos == new_shard.ids.len() - 1 {
        new_shard.last = id.to_string();
    }

    let new_value = encode_rkyv!(&new_shard)?;
    batch.insert(inverted, shard_key(word, target), &new_value);

    Ok(())
}

pub fn remove_from_posting_list(
    batch: &mut fjall::OwnedWriteBatch,
    inverted: &fjall::Keyspace,
    word: &str,
    id: &str,
) -> Result<(), AppError> {
    let indices = list_shard_indices(inverted, word)?;
    if indices.is_empty() {
        return Ok(());
    }

    let target = find_shard_for_id(inverted, word, id, &indices)?;
    let current = match load_posting_shard(inverted, word, target)? {
        Some(s) => s,
        None => return Ok(()),
    };

    let pos = match current.ids.binary_search(&id.to_string()) {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };

    let ran_first = pos == 0;
    let ran_last = pos == current.ids.len() - 1;
    let mut new_shard = current;
    new_shard.ids.remove(pos);

    let key = shard_key(word, target);
    if new_shard.ids.is_empty() {
        batch.remove(inverted, &key);
    } else {
        if ran_first {
            new_shard.first = new_shard.ids[0].clone();
        }
        if ran_last {
            new_shard.last = new_shard.ids.last().unwrap().clone();
        }
        let new_value = encode_rkyv!(&new_shard)?;
        batch.insert(inverted, &key, &new_value);
    }

    Ok(())
}

pub fn add_to_roaring_posting_list(
    batch: &mut fjall::OwnedWriteBatch,
    inverted: &fjall::Keyspace,
    word: &str,
    id: u64,
    max_roaring_shard_size: u64,
) -> Result<(), AppError> {
    let marker_key = word.as_bytes();
    if inverted.get(marker_key)?.is_none() {
        batch.insert(inverted, marker_key, []);
    }

    let indices = list_shard_indices(inverted, word)?;

    if indices.is_empty() {
        let mut bitmap = RoaringTreemap::new();
        bitmap.insert(id);
        let value = roaring_to_vec(&bitmap)?;
        batch.insert(inverted, shard_key(word, 0), &value);
        return Ok(());
    }

    let last_idx = *indices.last().unwrap();
    let last_key = shard_key(word, last_idx);
    let mut bitmap: RoaringTreemap = match inverted.get(&last_key)? {
        Some(data) => roaring_from_slice(&data)?,
        None => RoaringTreemap::new(),
    };

    if bitmap.len() < max_roaring_shard_size {
        bitmap.insert(id);
        let value = roaring_to_vec(&bitmap)?;
        batch.insert(inverted, &last_key, &value);
    } else {
        let mut new_bitmap = RoaringTreemap::new();
        new_bitmap.insert(id);
        let value = roaring_to_vec(&new_bitmap)?;
        batch.insert(inverted, shard_key(word, last_idx + 1), &value);
    }

    Ok(())
}

pub fn remove_from_roaring_posting_list(
    batch: &mut fjall::OwnedWriteBatch,
    inverted: &fjall::Keyspace,
    word: &str,
    id: u64,
) -> Result<(), AppError> {
    let indices = list_shard_indices(inverted, word)?;
    if indices.is_empty() {
        return Ok(());
    }

    for &shard_idx in &indices {
        let key = shard_key(word, shard_idx);
        let mut bitmap: RoaringTreemap = match inverted.get(&key)? {
            Some(data) => roaring_from_slice(&data)?,
            None => continue,
        };
        if !bitmap.contains(id) {
            continue;
        }
        bitmap.remove(id);
        if bitmap.is_empty() {
            batch.remove(inverted, &key);
        } else {
            let value = roaring_to_vec(&bitmap)?;
            batch.insert(inverted, &key, &value);
        }
        return Ok(());
    }

    Ok(())
}
