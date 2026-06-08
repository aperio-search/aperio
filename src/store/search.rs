use roaring::{MultiOps, RoaringTreemap};

use std::collections::BTreeSet;

use crate::error::AppError;

use super::DbBytes;
use super::config::{ArchivedPostingShard, PostingShard};
use super::fst::FSTPool;
use super::roaring_from_slice;
use super::tokenize;

pub struct SearchParams<'a> {
    pub inverted: DbBytes,
    pub txn: &'a heed::RoTxn<'a>,
    pub collection: &'a str,
    pub config_min_token_length: usize,
    pub config_max_token_length: usize,
    pub query: &'a str,
    pub sort_desc: bool,
    pub take: usize,
    pub after: Option<&'a str>,
    pub fuzzy_max_expansions: usize,
    pub fst_pool: Option<&'a FSTPool>,
}

struct WordIterState {
    cur_shard_data: Option<Vec<u8>>,
    cur_pos: usize,
}

impl WordIterState {
    fn new() -> Self {
        Self {
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
}

use super::posting_list;

pub fn roaring_search(params: SearchParams) -> Result<Vec<String>, AppError> {
    let SearchParams {
        inverted,
        txn,
        collection,
        config_min_token_length,
        config_max_token_length,
        query,
        sort_desc,
        take,
        after,
        fuzzy_max_expansions,
        fst_pool,
    } = params;
    let tokens: Vec<String> = tokenize(query, config_min_token_length, config_max_token_length)
        .into_iter()
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    // For each query token, collect all doc IDs (exact + fuzzy expansions).
    // Then intersect across tokens.
    let mut word_bitmaps: Vec<RoaringTreemap> = Vec::with_capacity(tokens.len());

    for token in &tokens {
        let exact_indices =
            posting_list::list_shard_indices(inverted, txn, collection, token).unwrap_or_default();

        let has_exact = !exact_indices.is_empty();

        // Load exact match bitmap
        let mut token_bitmap = RoaringTreemap::new();
        if has_exact {
            // Load all exact shards
            for &shard_idx in &exact_indices {
                let key = posting_list::shard_key(collection, token, shard_idx);
                if let Ok(Some(data)) = inverted.get(txn, key.as_slice())
                    && let Ok(bitmap) = roaring_from_slice(&data)
                {
                    token_bitmap |= &bitmap;
                }
            }
        }

        // Fuzzy expansion: if exact match has no results, try FST
        if !has_exact && let Some(pool) = fst_pool {
            let similar = pool.suggest_fuzzy(collection, token, fuzzy_max_expansions, None);
            for similar_term in &similar {
                let sim_indices =
                    posting_list::list_shard_indices(inverted, txn, collection, similar_term)
                        .unwrap_or_default();
                for &shard_idx in &sim_indices {
                    let key = posting_list::shard_key(collection, similar_term, shard_idx);
                    if let Ok(Some(data)) = inverted.get(txn, key.as_slice())
                        && let Ok(bitmap) = roaring_from_slice(&data)
                    {
                        token_bitmap |= &bitmap;
                    }
                }
            }
        }

        if token_bitmap.is_empty() {
            return Ok(Vec::new());
        }
        word_bitmaps.push(token_bitmap);
    }

    word_bitmaps.sort_by_key(|b| b.len());
    let bitmap = word_bitmaps.iter().intersection();
    // Parse the cursor strictly. Previously this used
    // `after.and_then(|a| a.parse::<u64>().ok())`, which silently swallowed
    // malformed values and restarted pagination from the beginning — every
    // page after a bad cursor would duplicate page 1. Number-id collections
    // use u64 cursors; anything else is a client error.
    let after_val = match after {
        Some(a) => Some(a.parse::<u64>().map_err(|parse_err| {
            AppError::BadRequest(format!(
                "invalid 'after' cursor '{a}': must be a non-negative integer for number-id collection: {parse_err}"
            ))
        })?),
        None => None,
    };
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
        config_max_token_length,
        query,
        sort_desc,
        take,
        after,
        fuzzy_max_expansions,
        fst_pool,
    } = params;
    let tokens: Vec<String> = tokenize(query, config_min_token_length, config_max_token_length)
        .into_iter()
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    // For each query token, build a unified sorted list of all unique doc IDs
    // (exact match + fuzzy expansions). Store them as "virtual shards" in a
    // map so WordIterState can reference them.
    #[derive(Default)]
    struct VirtualShard {
        ids: Vec<String>,
    }

    let mut virtual_shards: Vec<(String, VirtualShard)> = Vec::with_capacity(tokens.len());

    for token in &tokens {
        let mut all_ids = BTreeSet::new();

        let exact_indices =
            posting_list::list_shard_indices(inverted, txn, collection, token).unwrap_or_default();
        let has_exact = !exact_indices.is_empty();

        // Collect exact-match doc IDs
        if has_exact {
            for &shard_idx in &exact_indices {
                let key = posting_list::shard_key(collection, token, shard_idx);
                if let Ok(Some(data)) = inverted.get(txn, key.as_slice())
                    && let Ok(shard) =
                        rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&data)
                {
                    for id in shard.ids.iter() {
                        all_ids.insert(id.to_string());
                    }
                }
            }
        }

        // Fuzzy expansion: try FST if exact match has no results
        if !has_exact && let Some(pool) = fst_pool {
            let similar = pool.suggest_fuzzy(collection, token, fuzzy_max_expansions, None);
            for similar_term in &similar {
                let sim_indices =
                    posting_list::list_shard_indices(inverted, txn, collection, similar_term)
                        .unwrap_or_default();
                for &shard_idx in &sim_indices {
                    let key = posting_list::shard_key(collection, similar_term, shard_idx);
                    if let Ok(Some(data)) = inverted.get(txn, key.as_slice())
                        && let Ok(shard) =
                            rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&data)
                    {
                        for id in shard.ids.iter() {
                            all_ids.insert(id.to_string());
                        }
                    }
                }
            }
        }

        if all_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Build a virtual shard with sorted, deduped IDs
        let ids: Vec<String> = all_ids.into_iter().collect();
        virtual_shards.push((token.clone(), VirtualShard { ids }));
    }

    // Sort tokens by posting list size (rarest first) — but don't use term
    // itself after sort since we need the VirtualShard data to stay aligned
    virtual_shards.sort_by_key(|(_, shard)| shard.ids.len());

    // Encode each virtual shard with rkyv and set up iterators
    let mut encoded_shards: Vec<Vec<u8>> = Vec::new();
    for (_, shard) in &virtual_shards {
        let ps = PostingShard {
            ids: shard.ids.clone(),
        };
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&ps)
            .map(|av| av.to_vec())
            .map_err(|e| AppError::Internal(e.to_string()))?;
        encoded_shards.push(bytes);
    }

    // Sort by shard size (rarest first) — keep virtual_shards and encoded_shards
    // aligned by sorting all three together
    let mut combined: Vec<_> = virtual_shards
        .iter()
        .enumerate()
        .map(|(i, (word, shard))| (word.clone(), shard.ids.len(), i))
        .collect();
    combined.sort_by_key(|(_, len, _)| *len);

    let mut iters: Vec<WordIterState> = combined
        .iter()
        .map(|(_word, _, idx)| -> Result<WordIterState, AppError> {
            let mut state = WordIterState::new();
            state.cur_shard_data = Some(encoded_shards[*idx].clone());
            state.cur_pos = if sort_desc {
                // Point to last element for desc iteration
                if let Ok(shard) =
                    rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&encoded_shards[*idx])
                {
                    if !shard.ids.is_empty() {
                        shard.ids.len() - 1
                    } else {
                        0
                    }
                } else {
                    0
                }
            } else {
                0
            };
            Ok(state)
        })
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(cursor) = after {
        for (i, (_word, _, idx)) in combined.iter().enumerate() {
            skip_past_cursor_virtual(cursor, &mut iters[i], &encoded_shards[*idx], sort_desc)?;
        }
    }

    // The rest of the search loop uses real shard functions but the data is
    // already loaded in cur_shard_data. Since there's only one virtual shard
    // (index 0), advance_shard will stop immediately when exhausted.
    // We can reuse the existing functions by passing dummy keys, but we need
    // them to use the already-loaded cur_shard_data rather than reading LMDB.
    // To keep things simple, we implement the pivot loop directly here.

    let mut results: Vec<String> = Vec::with_capacity(take);

    loop {
        // Find the pivot (rarest ID across all iterators)
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

        // Check if all iterators have the pivot
        let mut all_have = true;

        for state in iters.iter_mut() {
            seek_in_shard(state, &pivot, sort_desc)?;
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
            for state in iters.iter_mut() {
                advance_in_virtual_shard(state, sort_desc);
            }
        }
    }

    Ok(results)
}

/// Seek within a single virtual shard (binary search on the sorted IDs).
fn seek_in_shard(state: &mut WordIterState, target: &str, desc: bool) -> Result<(), AppError> {
    match state.current() {
        None => return Ok(()),
        Some(id) => {
            let at_or_past = if desc { id <= target } else { id >= target };
            if at_or_past {
                return Ok(());
            }
        }
    }

    if let Some(ref data) = state.cur_shard_data
        && let Ok(shard) = rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(data)
    {
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

    // No more IDs — mark as exhausted
    state.cur_shard_data = None;
    state.cur_pos = 0;
    Ok(())
}

/// Advance by one position in a single virtual shard.
fn advance_in_virtual_shard(state: &mut WordIterState, desc: bool) {
    if desc {
        if state.cur_pos == 0 {
            state.cur_shard_data = None;
        } else {
            state.cur_pos -= 1;
        }
    } else if let Some(ref data) = state.cur_shard_data {
        let len = rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(data)
            .map(|s| s.ids.len())
            .unwrap_or(0);
        state.cur_pos += 1;
        if state.cur_pos >= len {
            state.cur_shard_data = None;
        }
    }
}

/// Apply the `after` cursor to a virtual shard.
fn skip_past_cursor_virtual(
    cursor: &str,
    state: &mut WordIterState,
    encoded: &[u8],
    desc: bool,
) -> Result<(), AppError> {
    if let Ok(shard) = rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(encoded) {
        let ids = &shard.ids;
        let len = ids.len();
        if len == 0 {
            state.cur_shard_data = None;
            state.cur_pos = 0;
            return Ok(());
        }

        // Binary search to find cursor position
        let mut lo = 0usize;
        let mut hi = len;
        while lo < hi {
            let mid = (lo + hi) / 2;
            match ids.get(mid) {
                Some(s) if s.as_str() < cursor => lo = mid + 1,
                _ => hi = mid,
            }
        }

        state.cur_shard_data = Some(encoded.to_vec());
        state.cur_pos = lo.min(len.saturating_sub(1));

        // Advance past the cursor
        loop {
            match state.current() {
                None => break,
                Some(id) => {
                    let should_skip = if desc { id >= cursor } else { id <= cursor };
                    if should_skip {
                        advance_in_virtual_shard(state, desc);
                    } else {
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}
