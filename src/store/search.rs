use fjall::Slice;
use rayon::prelude::*;
use roaring::{MultiOps, RoaringTreemap};

use crate::error::AppError;

use super::config::ArchivedPostingShard;
use super::roaring_from_slice;
use super::tokenize;

struct WordIterState {
    indices: Vec<usize>,
    shard_pos: isize,
    cur_slice: Option<Slice>,
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
            cur_slice: None,
            cur_pos: 0,
        }
    }

    fn current_archived(&self) -> Option<&ArchivedPostingShard> {
        rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(self.cur_slice.as_deref()?).ok()
    }

    fn current(&self) -> Option<&str> {
        let archived = self.current_archived()?;
        if self.cur_pos >= archived.ids.len() {
            return None;
        }
        archived.ids.get(self.cur_pos).map(|s| s.as_str())
    }

    fn ids_len(&self) -> usize {
        self.current_archived().map(|a| a.ids.len()).unwrap_or(0)
    }
}

use super::posting_list;

pub fn roaring_search(
    inverted: &fjall::Keyspace,
    config_min_token_length: usize,
    query: &str,
    sort_desc: bool,
    take: usize,
    after: Option<&str>,
) -> Result<Vec<String>, AppError> {
    let tokens: Vec<String> = tokenize(query, config_min_token_length)
        .into_iter()
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut word_shards: Vec<(String, Vec<usize>)> = tokens
        .par_iter()
        .map(|w| {
            let indices = posting_list::list_shard_indices(inverted, w).unwrap_or_default();
            (w.clone(), indices)
        })
        .collect();
    word_shards.sort_by_key(|a| a.1.len());
    if word_shards.first().is_none_or(|(_, idx)| idx.is_empty()) {
        return Ok(Vec::new());
    }

    let word_bitmaps: Vec<RoaringTreemap> = word_shards
        .par_iter()
        .map(|(word, indices)| -> Result<RoaringTreemap, AppError> {
            let mut word_bitmap = RoaringTreemap::new();
            for &shard_idx in indices {
                let key = posting_list::shard_key(word, shard_idx);
                if let Some(data) = inverted.get(&key)?
                    && let Ok(bitmap) = roaring_from_slice(&data)
                {
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

pub fn string_search(
    inverted: &fjall::Keyspace,
    config_min_token_length: usize,
    query: &str,
    sort_desc: bool,
    take: usize,
    after: Option<&str>,
) -> Result<Vec<String>, AppError> {
    let tokens: Vec<String> = tokenize(query, config_min_token_length)
        .into_iter()
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut word_shards: Vec<(String, Vec<usize>)> = tokens
        .par_iter()
        .map(|w| {
            let indices = posting_list::list_shard_indices(inverted, w).unwrap_or_default();
            (w.clone(), indices)
        })
        .collect();
    word_shards.sort_by_key(|a| a.1.len());
    if word_shards.first().is_none_or(|(_, idx)| idx.is_empty()) {
        return Ok(Vec::new());
    }

    let mut iters: Vec<WordIterState> = word_shards
        .par_iter()
        .map(|(word, indices)| -> Result<WordIterState, AppError> {
            let mut state = WordIterState::new(indices.clone(), sort_desc);
            if !sort_desc {
                load_first_shard(inverted, word, &mut state)?;
            } else {
                load_last_shard(inverted, word, &mut state)?;
            }
            Ok(state)
        })
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(cursor) = after {
        for (i, (word, _)) in word_shards.iter().enumerate() {
            skip_past_cursor(inverted, word, cursor, &mut iters[i], sort_desc)?;
        }
    }

    let mut results: Vec<String> = Vec::with_capacity(take);

    loop {
        let mut pivot: Option<String> = None;
        let mut any_exhausted = false;

        for state in &iters {
            match state.current() {
                None => {
                    any_exhausted = true;
                }
                Some(id) => match &pivot {
                    None => {
                        pivot = Some(id.to_string());
                    }
                    Some(p) => {
                        let take_this = if sort_desc {
                            id < p.as_str()
                        } else {
                            id > p.as_str()
                        };
                        if take_this {
                            pivot = Some(id.to_string());
                        }
                    }
                },
            }
        }

        if pivot.is_none() || any_exhausted {
            break;
        }

        let pivot = pivot.unwrap();
        let mut all_have = true;

        for (i, state) in iters.iter_mut().enumerate() {
            seek_to(inverted, &word_shards[i].0, state, &pivot, sort_desc)?;
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
                advance_iter(inverted, &word_shards[i].0, state, sort_desc)?;
            }
        }
    }

    Ok(results)
}

fn load_first_shard(
    inverted: &fjall::Keyspace,
    word: &str,
    state: &mut WordIterState,
) -> Result<(), AppError> {
    while (state.shard_pos as usize) < state.indices.len() {
        let idx = state.indices[state.shard_pos as usize];
        let key = posting_list::shard_key(word, idx);
        if let Some(data) = inverted.get(&key)? {
            let non_empty = rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&data)
                .map(|a| !a.ids.is_empty())
                .unwrap_or(false);
            if non_empty {
                state.cur_slice = Some(data);
                state.cur_pos = 0;
                return Ok(());
            }
        }
        state.shard_pos += 1;
    }
    state.cur_slice = None;
    state.cur_pos = 0;
    Ok(())
}

fn load_last_shard(
    inverted: &fjall::Keyspace,
    word: &str,
    state: &mut WordIterState,
) -> Result<(), AppError> {
    while state.shard_pos >= 0 {
        let idx = state.indices[state.shard_pos as usize];
        let key = posting_list::shard_key(word, idx);
        if let Some(data) = inverted.get(&key)? {
            let maybe_len = rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&data)
                .map(|a| a.ids.len())
                .unwrap_or(0);
            if maybe_len > 0 {
                state.cur_slice = Some(data);
                state.cur_pos = maybe_len.saturating_sub(1);
                return Ok(());
            }
        }
        state.shard_pos -= 1;
    }
    state.cur_slice = None;
    state.cur_pos = 0;
    Ok(())
}

fn advance_shard(
    inverted: &fjall::Keyspace,
    word: &str,
    state: &mut WordIterState,
    desc: bool,
) -> Result<(), AppError> {
    loop {
        if desc {
            state.shard_pos -= 1;
            if state.shard_pos < 0 {
                state.cur_slice = None;
                state.cur_pos = 0;
                return Ok(());
            }
        } else {
            state.shard_pos += 1;
            if (state.shard_pos as usize) >= state.indices.len() {
                state.cur_slice = None;
                state.cur_pos = 0;
                return Ok(());
            }
        }
        let idx = state.indices[state.shard_pos as usize];
        let key = posting_list::shard_key(word, idx);
        if let Some(data) = inverted.get(&key)? {
            let maybe_len = rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&data)
                .map(|a| a.ids.len())
                .unwrap_or(0);
            if maybe_len > 0 {
                state.cur_slice = Some(data);
                state.cur_pos = if desc { maybe_len.saturating_sub(1) } else { 0 };
                return Ok(());
            }
        }
    }
}

fn advance_iter(
    inverted: &fjall::Keyspace,
    word: &str,
    state: &mut WordIterState,
    desc: bool,
) -> Result<(), AppError> {
    if desc {
        if state.cur_pos == 0 {
            return advance_shard(inverted, word, state, desc);
        }
        state.cur_pos -= 1;
    } else {
        state.cur_pos += 1;
        if state.cur_pos >= state.ids_len() {
            return advance_shard(inverted, word, state, desc);
        }
    }
    Ok(())
}

fn seek_to(
    inverted: &fjall::Keyspace,
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

        if let Some(archived) = state.current_archived() {
            let ids = &archived.ids;
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

        advance_shard(inverted, word, state, desc)?;
    }
}

fn skip_past_cursor(
    inverted: &fjall::Keyspace,
    word: &str,
    cursor: &str,
    state: &mut WordIterState,
    desc: bool,
) -> Result<(), AppError> {
    if state.indices.is_empty() {
        return Ok(());
    }

    let shard_idx = posting_list::find_shard_for_id(inverted, word, cursor, &state.indices)?;
    let pos_in_indices = state
        .indices
        .iter()
        .position(|&i| i == shard_idx)
        .unwrap_or(state.indices.len().saturating_sub(1));
    state.shard_pos = pos_in_indices as isize;

    let key = posting_list::shard_key(word, shard_idx);
    match inverted.get(&key)? {
        Some(data) => {
            let (pos, ids_len) =
                match rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&data) {
                    Ok(archived) => {
                        let ids = &archived.ids;
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
                        (lo, ids.len())
                    }
                    Err(_) => {
                        state.cur_slice = None;
                        state.cur_pos = 0;
                        return Ok(());
                    }
                };
            state.cur_slice = Some(data);
            state.cur_pos = pos.min(ids_len.saturating_sub(1));
        }
        None => {
            state.cur_slice = None;
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
                    advance_iter(inverted, word, state, desc)?;
                } else {
                    break;
                }
            }
        }
    }

    Ok(())
}
