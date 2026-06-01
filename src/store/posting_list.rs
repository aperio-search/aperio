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
            let shard: PostingShard = decode_rkyv!(PostingShard, &data)
                .unwrap_or_else(|_| PostingShard { ids: Vec::new() });
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
        let shard_first = shard.ids.first().map(|s| s.as_str()).unwrap_or("");
        let shard_last = shard.ids.last().map(|s| s.as_str()).unwrap_or("");
        if *id < *shard_first {
            if mid == 0 {
                return Ok(indices[0]);
            }
            hi = mid - 1;
        } else if *id > *shard_last {
            lo = mid + 1;
        } else {
            return Ok(indices[mid]);
        }
    }
    Ok(indices[lo.min(indices.len().saturating_sub(1))])
}

pub fn add_to_posting_list(
    inverted: &fjall::Keyspace,
    word: &str,
    id: &str,
    max_string_shard_size: usize,
) -> Result<(), AppError> {
    let indices = list_shard_indices(inverted, word)?;

    if indices.is_empty() {
        let shard = PostingShard {
            ids: vec![id.to_string()],
        };
        let value = encode_rkyv!(&shard)?;
        inverted.insert(shard_key(word, 0), &value)?;
        return Ok(());
    }

    let last_idx = *indices.last().unwrap();
    let last_shard = load_posting_shard(inverted, word, last_idx)?
        .unwrap_or_else(|| PostingShard { ids: Vec::new() });

    if last_shard
        .ids
        .last()
        .map(|s| id > s.as_str())
        .unwrap_or(true)
    {
        if last_shard.ids.len() < max_string_shard_size {
            if last_shard.ids.binary_search(&id.to_string()).is_ok() {
                return Ok(());
            }
            let mut new_shard = last_shard;
            new_shard.ids.push(id.to_string());
            let new_value = encode_rkyv!(&new_shard)?;
            inverted.insert(shard_key(word, last_idx), &new_value)?;
        } else {
            let shard = PostingShard {
                ids: vec![id.to_string()],
            };
            let value = encode_rkyv!(&shard)?;
            inverted.insert(shard_key(word, last_idx + 1), &value)?;
        }
        return Ok(());
    }

    let target = find_shard_for_id(inverted, word, id, &indices)?;
    let current = load_posting_shard(inverted, word, target)?
        .unwrap_or_else(|| PostingShard { ids: Vec::new() });

    if current.ids.binary_search(&id.to_string()).is_ok() {
        return Ok(());
    }

    let pos = current.ids.binary_search(&id.to_string()).unwrap_err();
    let mut new_shard = current;
    new_shard.ids.insert(pos, id.to_string());

    let new_value = encode_rkyv!(&new_shard)?;
    inverted.insert(shard_key(word, target), &new_value)?;

    Ok(())
}

pub fn remove_from_posting_list(
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

    let mut new_shard = current;
    new_shard.ids.remove(pos);

    let key = shard_key(word, target);
    if new_shard.ids.is_empty() {
        inverted.remove(&key)?;
    } else {
        let new_value = encode_rkyv!(&new_shard)?;
        inverted.insert(&key, &new_value)?;
    }

    Ok(())
}

pub fn add_to_roaring_posting_list(
    inverted: &fjall::Keyspace,
    word: &str,
    id: u64,
    max_roaring_shard_size: u64,
) -> Result<(), AppError> {
    let indices = list_shard_indices(inverted, word)?;

    if indices.is_empty() {
        let mut bitmap = RoaringTreemap::new();
        bitmap.insert(id);
        let value = roaring_to_vec(&bitmap)?;
        inverted.insert(shard_key(word, 0), &value)?;
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
        inverted.insert(&last_key, &value)?;
    } else {
        let mut new_bitmap = RoaringTreemap::new();
        new_bitmap.insert(id);
        let value = roaring_to_vec(&new_bitmap)?;
        inverted.insert(shard_key(word, last_idx + 1), &value)?;
    }

    Ok(())
}

pub fn remove_from_roaring_posting_list(
    inverted: &fjall::Keyspace,
    word: &str,
    id: u64,
) -> Result<(), AppError> {
    let indices = list_shard_indices(inverted, word)?;
    if indices.is_empty() {
        return Ok(());
    }

    let target = {
        let mut lo = 0usize;
        let mut hi = indices.len().saturating_sub(1);
        loop {
            if lo > hi {
                break indices[lo.min(indices.len().saturating_sub(1))];
            }
            let mid = (lo + hi) / 2;
            let key = shard_key(word, indices[mid]);
            let data = match inverted.get(&key)? {
                Some(d) => d,
                None => {
                    lo = mid + 1;
                    continue;
                }
            };
            let bitmap = roaring_from_slice(&data)?;
            let shard_first = bitmap.min().unwrap_or(0);
            let shard_last = bitmap.max().unwrap_or(0);
            if id < shard_first {
                if mid == 0 {
                    break indices[0];
                }
                hi = mid - 1;
            } else if id > shard_last {
                lo = mid + 1;
            } else {
                break indices[mid];
            }
        }
    };

    let key = shard_key(word, target);
    let data = match inverted.get(&key)? {
        Some(d) => d,
        None => return Ok(()),
    };

    let mut bitmap = roaring_from_slice(&data)?;
    if !bitmap.contains(id) {
        return Ok(());
    }

    bitmap.remove(id);

    if bitmap.is_empty() {
        inverted.remove(&key)?;
    } else {
        let value = roaring_to_vec(&bitmap)?;
        inverted.insert(&key, &value)?;
    }

    Ok(())
}
