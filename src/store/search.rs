use rayon::prelude::*;
use roaring::{MultiOps, RoaringTreemap};

use crate::error::AppError;

use super::DbBytes;
use super::config::ArchivedPostingShard;
use super::roaring_from_slice;
use super::tokenize;

pub struct SearchParams<'a> {
    pub inverted: DbBytes,
    pub txn: &'a heed::RoTxn<'a>,
    pub collection: &'a str,
    pub config_min_token_length: usize,
    pub query: &'a str,
    pub sort_desc: bool,
    pub take: usize,
    pub after: Option<&'a str>,
}

struct WordIterState {
    indices: Vec<usize>,
    shard_pos: isize,
    cur_shard_data: Option<Vec<u8>>,
    cur_pos: usize,
}

impl WordIterState {
    fn new(indices: Vec<usize>, desc: bool) -> Self {
        let shard_pos = if desc {
            indices.len().saturating_sub(1) as isize
        } else {
            0
        };
        Self {
            indices,
            shard_pos,
            cur_shard_data: None,
            cur_pos: 0,
        }
    }

    fn archived_shard(&self) -> Option<&ArchivedPostingShard> {
        let data = self.cur_shard_data.as_ref()?;
        rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(data).ok()
    }

    fn current(&self) -> Option<&str> {
        let shard = self.archived_shard()?;
        if self.cur_pos >= shard.ids.len() {
            return None;
        }
        shard.ids.get(self.cur_pos).map(|s| s.as_str())
    }

    fn ids_len(&self) -> usize {
        self.archived_shard().map(|s| s.ids.len()).unwrap_or(0)
    }
}

use super::posting_list;

pub fn roaring_search(params: SearchParams) -> Result<Vec<String>, AppError> {
    let SearchParams {
        inverted,
        txn,
        collection,
        config_min_token_length,
        query,
        sort_desc,
        take,
        after,
    } = params;
    let tokens: Vec<String> = tokenize(query, config_min_token_length)
        .into_iter()
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut word_shards: Vec<(String, Vec<usize>)> = tokens
        .iter()
        .map(|w| {
            let indices =
                posting_list::list_shard_indices(inverted, txn, collection, w).unwrap_or_default();
            (w.clone(), indices)
        })
        .collect();
    word_shards.sort_by_key(|a| a.1.len());
    if word_shards.first().is_none_or(|(_, idx)| idx.is_empty()) {
        return Ok(Vec::new());
    }

    let word_shard_data: Vec<Vec<Vec<u8>>> = word_shards
        .iter()
        .map(|(word, indices)| {
            indices
                .iter()
                .filter_map(|&shard_idx| {
                    let key = posting_list::shard_key(collection, word, shard_idx);
                    match inverted.get(txn, key.as_slice()) {
                        Ok(Some(v)) => Some(v),
                        _ => None,
                    }
                })
                .collect()
        })
        .collect();

    let word_bitmaps: Vec<RoaringTreemap> = word_shard_data
        .par_iter()
        .map(|shard_data| -> Result<RoaringTreemap, AppError> {
            let mut word_bitmap = RoaringTreemap::new();
            for data in shard_data {
                if let Ok(bitmap) = roaring_from_slice(data) {
                    word_bitmap |= &bitmap;
                }
            }
            Ok(word_bitmap)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let bitmap = word_bitmaps.iter().intersection();

    let after_val = after.and_then(|a| a.parse::<u64>().ok());
    let iter: Box<dyn Iterator<Item = u64>> = if sort_desc {
        Box::new(bitmap.into_iter().rev())
    } else {
        Box::new(bitmap.into_iter())
    };

    let mut results: Vec<String> = Vec::with_capacity(take);
    for id in iter {
        if let Some(cursor) = after_val {
            if sort_desc && id >= cursor {
                continue;
            }
            if !sort_desc && id <= cursor {
                continue;
            }
        }
        results.push(id.to_string());
        if results.len() >= take {
            break;
        }
    }

    Ok(results)
}

pub fn string_search(params: SearchParams) -> Result<Vec<String>, AppError> {
    let SearchParams {
        inverted,
        txn,
        collection,
        config_min_token_length,
        query,
        sort_desc,
        take,
        after,
    } = params;
    let tokens: Vec<String> = tokenize(query, config_min_token_length)
        .into_iter()
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut word_shards: Vec<(String, Vec<usize>)> = tokens
        .iter()
        .map(|w| {
            let indices =
                posting_list::list_shard_indices(inverted, txn, collection, w).unwrap_or_default();
            (w.clone(), indices)
        })
        .collect();
    word_shards.sort_by_key(|a| a.1.len());
    if word_shards.first().is_none_or(|(_, idx)| idx.is_empty()) {
        return Ok(Vec::new());
    }

    let mut iters: Vec<WordIterState> = word_shards
        .iter()
        .map(|(word, indices)| -> Result<WordIterState, AppError> {
            let mut state = WordIterState::new(indices.clone(), sort_desc);
            if !sort_desc {
                load_first_shard(inverted, txn, collection, word, &mut state)?;
            } else {
                load_last_shard(inverted, txn, collection, word, &mut state)?;
            }
            Ok(state)
        })
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(cursor) = after {
        for (i, (word, _)) in word_shards.iter().enumerate() {
            skip_past_cursor(
                inverted,
                txn,
                collection,
                word,
                cursor,
                &mut iters[i],
                sort_desc,
            )?;
        }
    }

    let mut results: Vec<String> = Vec::with_capacity(take);

    loop {
        let pivot_str = {
            let mut best: Option<&str> = None;
            let mut any_exhausted = false;

            for state in &iters {
                match state.current() {
                    None => {
                        any_exhausted = true;
                        break;
                    }
                    Some(id) => match best {
                        None => best = Some(id),
                        Some(b) => {
                            let is_better = if sort_desc { id < b } else { id > b };
                            if is_better {
                                best = Some(id);
                            }
                        }
                    },
                }
            }

            if any_exhausted {
                None
            } else {
                best.map(|s| s.to_string())
            }
        };

        let pivot = match pivot_str {
            Some(p) => p,
            None => break,
        };

        let mut all_have = true;

        for (i, state) in iters.iter_mut().enumerate() {
            seek_to(
                inverted,
                txn,
                collection,
                &word_shards[i].0,
                state,
                &pivot,
                sort_desc,
            )?;
            match state.current() {
                None => {
                    all_have = false;
                    break;
                }
                Some(id) => {
                    if id != pivot.as_str() {
                        all_have = false;
                        break;
                    }
                }
            }
        }

        if all_have {
            results.push(pivot);
            if results.len() >= take {
                break;
            }
            for (i, state) in iters.iter_mut().enumerate() {
                advance_iter(
                    inverted,
                    txn,
                    collection,
                    &word_shards[i].0,
                    state,
                    sort_desc,
                )?;
            }
        }
    }

    Ok(results)
}

fn get_shard_data(
    inverted: DbBytes,
    txn: &heed::RoTxn,
    collection: &str,
    word: &str,
    shard_idx: usize,
) -> Result<Option<Vec<u8>>, AppError> {
    let key = posting_list::shard_key(collection, word, shard_idx);
    Ok(inverted.get(txn, key.as_slice())?)
}

fn load_first_shard(
    inverted: DbBytes,
    txn: &heed::RoTxn,
    collection: &str,
    word: &str,
    state: &mut WordIterState,
) -> Result<(), AppError> {
    while (state.shard_pos as usize) < state.indices.len() {
        let idx = state.indices[state.shard_pos as usize];
        if let Some(data) = get_shard_data(inverted, txn, collection, word, idx)? {
            state.cur_shard_data = Some(data);
            if state.archived_shard().is_some_and(|s| !s.ids.is_empty()) {
                state.cur_pos = 0;
                return Ok(());
            }
            state.cur_shard_data = None;
        }
        state.shard_pos += 1;
    }
    state.cur_shard_data = None;
    state.cur_pos = 0;
    Ok(())
}

fn load_last_shard(
    inverted: DbBytes,
    txn: &heed::RoTxn,
    collection: &str,
    word: &str,
    state: &mut WordIterState,
) -> Result<(), AppError> {
    while state.shard_pos >= 0 {
        let idx = state.indices[state.shard_pos as usize];
        if let Some(data) = get_shard_data(inverted, txn, collection, word, idx)? {
            state.cur_shard_data = Some(data);
            let maybe_len = state.ids_len();
            if maybe_len > 0 {
                state.cur_pos = maybe_len.saturating_sub(1);
                return Ok(());
            }
            state.cur_shard_data = None;
        }
        state.shard_pos -= 1;
    }
    state.cur_shard_data = None;
    state.cur_pos = 0;
    Ok(())
}

fn advance_shard(
    inverted: DbBytes,
    txn: &heed::RoTxn,
    collection: &str,
    word: &str,
    state: &mut WordIterState,
    desc: bool,
) -> Result<(), AppError> {
    loop {
        if desc {
            state.shard_pos -= 1;
            if state.shard_pos < 0 {
                state.cur_shard_data = None;
                state.cur_pos = 0;
                return Ok(());
            }
        } else {
            state.shard_pos += 1;
            if (state.shard_pos as usize) >= state.indices.len() {
                state.cur_shard_data = None;
                state.cur_pos = 0;
                return Ok(());
            }
        }
        let idx = state.indices[state.shard_pos as usize];
        if let Some(data) = get_shard_data(inverted, txn, collection, word, idx)? {
            state.cur_shard_data = Some(data);
            let maybe_len = state.ids_len();
            if maybe_len > 0 {
                state.cur_pos = if desc { maybe_len.saturating_sub(1) } else { 0 };
                return Ok(());
            }
            state.cur_shard_data = None;
        }
    }
}

fn advance_iter(
    inverted: DbBytes,
    txn: &heed::RoTxn,
    collection: &str,
    word: &str,
    state: &mut WordIterState,
    desc: bool,
) -> Result<(), AppError> {
    if desc {
        if state.cur_pos == 0 {
            return advance_shard(inverted, txn, collection, word, state, desc);
        }
        state.cur_pos -= 1;
    } else {
        state.cur_pos += 1;
        if state.cur_pos >= state.ids_len() {
            return advance_shard(inverted, txn, collection, word, state, desc);
        }
    }
    Ok(())
}

fn seek_to(
    inverted: DbBytes,
    txn: &heed::RoTxn,
    collection: &str,
    word: &str,
    state: &mut WordIterState,
    target: &str,
    desc: bool,
) -> Result<(), AppError> {
    loop {
        match state.current() {
            None => return Ok(()),
            Some(id) => {
                let at_or_past = if desc { id <= target } else { id >= target };
                if at_or_past {
                    return Ok(());
                }
            }
        }

        if let Some(shard) = state.archived_shard() {
            let ids = &shard.ids;
            let len = ids.len();
            if desc {
                let mut lo = 0usize;
                let mut hi = len;
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    match ids.get(mid) {
                        Some(s) if s.as_str() <= target => lo = mid + 1,
                        _ => hi = mid,
                    }
                }
                if lo > 0 {
                    state.cur_pos = lo - 1;
                    return Ok(());
                }
            } else {
                let mut lo = 0usize;
                let mut hi = len;
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    match ids.get(mid) {
                        Some(s) if s.as_str() < target => lo = mid + 1,
                        _ => hi = mid,
                    }
                }
                if lo < len {
                    state.cur_pos = lo;
                    return Ok(());
                }
            }
        }

        advance_shard(inverted, txn, collection, word, state, desc)?;
    }
}

fn skip_past_cursor(
    inverted: DbBytes,
    txn: &heed::RoTxn,
    collection: &str,
    word: &str,
    cursor: &str,
    state: &mut WordIterState,
    desc: bool,
) -> Result<(), AppError> {
    if state.indices.is_empty() {
        return Ok(());
    }

    let shard_idx =
        posting_list::find_shard_for_id(inverted, txn, collection, word, cursor, &state.indices)?;
    let pos_in_indices = state
        .indices
        .iter()
        .position(|&i| i == shard_idx)
        .unwrap_or(state.indices.len().saturating_sub(1));
    state.shard_pos = pos_in_indices as isize;

    match get_shard_data(inverted, txn, collection, word, shard_idx)? {
        Some(data) => {
            let new_cur_pos = {
                let archived = rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&data);
                let shard = match archived {
                    Ok(s) => s,
                    Err(_) => {
                        state.cur_shard_data = None;
                        state.cur_pos = 0;
                        return Ok(());
                    }
                };
                let ids = &shard.ids;
                let mut lo = 0;
                let mut hi = ids.len();
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    match ids.get(mid) {
                        Some(s) if s.as_str() < cursor => {
                            lo = mid + 1;
                        }
                        _ => {
                            hi = mid;
                        }
                    }
                }
                lo.min(ids.len().saturating_sub(1))
            };
            state.cur_shard_data = Some(data);
            state.cur_pos = new_cur_pos;
        }
        None => {
            state.cur_shard_data = None;
            state.cur_pos = 0;
            return Ok(());
        }
    }

    loop {
        let cur = state.current();
        match cur {
            None => break,
            Some(id) => {
                let should_skip = if desc { id >= cursor } else { id <= cursor };
                if should_skip {
                    advance_iter(inverted, txn, collection, word, state, desc)?;
                } else {
                    break;
                }
            }
        }
    }

    Ok(())
}
