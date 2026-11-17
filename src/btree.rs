use std::ops::Bound;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub ts: u64,
    pub value: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct LeafEntry {
    pub key: Vec<u8>,
    /// Newest version first (descending `ts`).
    pub versions: Vec<Version>,
}

#[derive(Debug, Clone)]
enum Node {
    Internal {
        keys: Vec<Vec<u8>>,
        children: Vec<usize>,
    },
    Leaf {
        entries: Vec<LeafEntry>,
        next: Option<usize>,
    },
}

/// Arena-allocated in-memory B+tree mapping keys to MVCC version chains.
#[derive(Debug, Clone)]
pub struct BPlusTree {
    order: usize,
    nodes: Vec<Node>,
    root: usize,
    first_leaf: usize,
    num_keys: usize,
    num_versions: usize,
}

impl BPlusTree {
    pub fn new(order: usize) -> Self {
        let order = order.max(4);
        let root_node = Node::Leaf {
            entries: Vec::new(),
            next: None,
        };
        Self {
            order,
            nodes: vec![root_node],
            root: 0,
            first_leaf: 0,
            num_keys: 0,
            num_versions: 0,
        }
    }

    pub const fn key_count(&self) -> usize {
        self.num_keys
    }

    pub const fn version_count(&self) -> usize {
        self.num_versions
    }

    fn find_leaf(&self, key: &[u8]) -> usize {
        let mut idx = self.root;
        loop {
            match &self.nodes[idx] {
                Node::Internal { keys, children } => {
                    let pos = keys.partition_point(|k| k.as_slice() <= key);
                    idx = children[pos];
                }
                Node::Leaf { .. } => return idx,
            }
        }
    }

    pub fn latest_ts(&self, key: &[u8]) -> Option<u64> {
        let leaf_idx = self.find_leaf(key);
        if let Node::Leaf { entries, .. } = &self.nodes[leaf_idx]
            && let Ok(pos) = entries.binary_search_by(|e| e.key.as_slice().cmp(key))
        {
            return entries[pos].versions.first().map(|v| v.ts);
        }
        None
    }

    pub fn get_visible(&self, key: &[u8], snapshot: u64) -> Option<Vec<u8>> {
        let leaf_idx = self.find_leaf(key);
        if let Node::Leaf { entries, .. } = &self.nodes[leaf_idx]
            && let Ok(pos) = entries.binary_search_by(|e| e.key.as_slice().cmp(key))
        {
            for v in &entries[pos].versions {
                if v.ts <= snapshot {
                    return v.value.clone();
                }
            }
        }
        None
    }

    pub fn scan_visible(
        &self,
        range: (Bound<&Vec<u8>>, Bound<&Vec<u8>>),
        snapshot: u64,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut leaf_idx = match range.0 {
            Bound::Included(k) | Bound::Excluded(k) => self.find_leaf(k),
            Bound::Unbounded => self.first_leaf,
        };

        let mut out = Vec::new();
        while let Node::Leaf { entries, next } = &self.nodes[leaf_idx] {
            for entry in entries {
                match range.0 {
                    Bound::Included(start) if &entry.key < start => continue,
                    Bound::Excluded(start) if &entry.key <= start => continue,
                    _ => {}
                }
                match range.1 {
                    Bound::Included(end) if &entry.key > end => return out,
                    Bound::Excluded(end) if &entry.key >= end => return out,
                    _ => {}
                }
                if let Some(v) = entry.versions.iter().find(|v| v.ts <= snapshot)
                    && let Some(val) = &v.value
                {
                    out.push((entry.key.clone(), val.clone()));
                }
            }

            if let Some(next_idx) = *next {
                leaf_idx = next_idx;
            } else {
                break;
            }
        }
        out
    }

    pub fn latest_live_versions(&self, max_ts: u64) -> Vec<(Vec<u8>, Version)> {
        let mut out = Vec::new();
        let mut leaf_idx = self.first_leaf;
        while let Node::Leaf { entries, next } = &self.nodes[leaf_idx] {
            for entry in entries {
                if let Some(v) = entry.versions.iter().find(|v| v.ts <= max_ts)
                    && v.value.is_some()
                {
                    out.push((entry.key.clone(), v.clone()));
                }
            }
            if let Some(next_idx) = *next {
                leaf_idx = next_idx;
            } else {
                break;
            }
        }
        out
    }

    pub fn insert_version(&mut self, key: Vec<u8>, version: Version) {
        let root = self.root;
        if let Some((sep_key, right_idx)) = self.insert_rec(root, key, version) {
            let new_root = Node::Internal {
                keys: vec![sep_key],
                children: vec![root, right_idx],
            };
            let new_root_idx = self.nodes.len();
            self.nodes.push(new_root);
            self.root = new_root_idx;
        }
    }

    fn insert_rec(
        &mut self,
        node_idx: usize,
        key: Vec<u8>,
        version: Version,
    ) -> Option<(Vec<u8>, usize)> {
        match &mut self.nodes[node_idx] {
            Node::Leaf { entries, next } => {
                match entries.binary_search_by(|e| e.key.cmp(&key)) {
                    Ok(pos) => {
                        let vers = &mut entries[pos].versions;
                        if let Some(first) = vers.first_mut()
                            && first.ts == version.ts
                        {
                            *first = version;
                        } else {
                            let ins = vers.partition_point(|v| v.ts > version.ts);
                            vers.insert(ins, version);
                            self.num_versions += 1;
                        }
                        return None;
                    }
                    Err(pos) => {
                        entries.insert(
                            pos,
                            LeafEntry {
                                key,
                                versions: vec![version],
                            },
                        );
                        self.num_keys += 1;
                        self.num_versions += 1;
                    }
                }

                if entries.len() < self.order {
                    return None;
                }

                let mid = entries.len() / 2;
                let right_entries = entries.split_off(mid);
                let sep_key = right_entries[0].key.clone();
                let old_next = *next;
                let right_idx = self.nodes.len();
                if let Node::Leaf { next, .. } = &mut self.nodes[node_idx] {
                    *next = Some(right_idx);
                }
                self.nodes.push(Node::Leaf {
                    entries: right_entries,
                    next: old_next,
                });
                Some((sep_key, right_idx))
            }
            Node::Internal { keys, children } => {
                let pos = keys.partition_point(|k| k <= &key);
                let child_idx = children[pos];
                let split = self.insert_rec(child_idx, key, version)?;

                let Node::Internal { keys, children } = &mut self.nodes[node_idx] else {
                    unreachable!()
                };
                keys.insert(pos, split.0);
                children.insert(pos + 1, split.1);

                if children.len() <= self.order {
                    return None;
                }

                let mid = keys.len() / 2;
                let right_keys = keys.split_off(mid + 1);
                let up_key = keys.pop().unwrap_or_default();
                let right_children = children.split_off(mid + 1);
                let right_idx = self.nodes.len();
                self.nodes.push(Node::Internal {
                    keys: right_keys,
                    children: right_children,
                });
                Some((up_key, right_idx))
            }
        }
    }

    /// Garbage-collects `key` given `min_snapshot` (the oldest active snapshot, or `last_ts` if
    /// no transactions are active).
    ///
    /// Retains all versions with `ts > min_snapshot` plus at most one version with
    /// `ts <= min_snapshot` (the newest such version). If the only remaining version is a
    /// tombstone (`value.is_none()`), removes the key from the leaf entirely.
    /// Returns `true` if GC for this key is fully complete (at most 1 version remaining).
    pub fn gc_key(&mut self, key: &[u8], min_snapshot: u64) -> bool {
        let leaf_idx = self.find_leaf(key);
        let Node::Leaf { entries, .. } = &mut self.nodes[leaf_idx] else {
            return true;
        };
        let Ok(pos) = entries.binary_search_by(|e| e.key.as_slice().cmp(key)) else {
            return true;
        };

        let versions = &mut entries[pos].versions;
        let mut seen_at_or_below = false;
        let before_len = versions.len();
        versions.retain(|v| {
            if v.ts > min_snapshot {
                true
            } else if !seen_at_or_below {
                seen_at_or_below = true;
                true
            } else {
                false
            }
        });
        let removed_versions = before_len - versions.len();
        self.num_versions -= removed_versions;

        if versions.len() == 1 && versions[0].value.is_none() {
            entries.remove(pos);
            self.num_keys -= 1;
            self.num_versions -= 1;
            return true;
        }

        versions.len() <= 1
    }

    pub fn validate_invariants(&self) -> Result<(), &'static str> {
        let mut leaf_depths = Vec::new();
        self.check_node(self.root, 0, None, None, &mut leaf_depths)?;
        if let Some(&first_depth) = leaf_depths.first()
            && !leaf_depths.iter().all(|&d| d == first_depth)
        {
            return Err("leaves are not all at the same depth");
        }
        Ok(())
    }

    fn check_node<'a>(
        &'a self,
        node_idx: usize,
        depth: usize,
        lower: Option<&'a [u8]>,
        upper: Option<&'a [u8]>,
        leaf_depths: &mut Vec<usize>,
    ) -> Result<(), &'static str> {
        match &self.nodes[node_idx] {
            Node::Leaf { entries, .. } => {
                leaf_depths.push(depth);
                for pair in entries.windows(2) {
                    if pair[0].key >= pair[1].key {
                        return Err("leaf keys not strictly sorted");
                    }
                }
                for e in entries {
                    if let Some(lo) = lower
                        && e.key.as_slice() < lo
                    {
                        return Err("leaf key below lower separator bound");
                    }
                    if let Some(hi) = upper
                        && e.key.as_slice() >= hi
                    {
                        return Err("leaf key at or above upper separator bound");
                    }
                }
                Ok(())
            }
            Node::Internal { keys, children } => {
                if children.len() != keys.len() + 1 {
                    return Err("internal node children/keys count mismatch");
                }
                for pair in keys.windows(2) {
                    if pair[0] >= pair[1] {
                        return Err("internal separator keys not sorted");
                    }
                }
                for (i, &child) in children.iter().enumerate() {
                    let child_lo = if i == 0 {
                        lower
                    } else {
                        Some(keys[i - 1].as_slice())
                    };
                    let child_hi = if i == keys.len() {
                        upper
                    } else {
                        Some(keys[i].as_slice())
                    };
                    self.check_node(child, depth + 1, child_lo, child_hi, leaf_depths)?;
                }
                Ok(())
            }
        }
    }
}
