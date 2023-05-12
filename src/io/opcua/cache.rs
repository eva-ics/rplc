use opcua::client::prelude::{NodeId, Variant};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::Duration;
use ttl_cache::TtlCache;

const OPC_CACHE_MAX_CAPACITY: usize = 100_000;

#[allow(clippy::module_name_repetitions)]
pub struct OpcCache {
    cache: Mutex<TtlCache<NodeId, Variant>>,
    ttl: Option<Duration>,
}

impl OpcCache {
    pub fn new(ttl: Option<Duration>) -> Self {
        Self {
            cache: Mutex::new(TtlCache::new(OPC_CACHE_MAX_CAPACITY)),
            ttl,
        }
    }
    pub fn retain_map_modified(&self, states: &mut HashMap<&NodeId, Variant>) {
        if let Some(ttl) = self.ttl {
            let mut cache = self.cache.lock();
            states.retain(|node_id, raw| {
                if let Some(cached) = cache.get(node_id) {
                    cached != raw
                } else {
                    true
                }
            });
            // cache kept ones
            for (oid, raw) in states {
                cache.insert((*oid).clone(), raw.clone(), ttl);
            }
        }
    }
}
