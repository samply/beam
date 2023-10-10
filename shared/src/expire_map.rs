use std::{hash::Hash, ops::Deref, time::Duration};

use dashmap::DashMap;
use tokio::time::Instant;

pub struct LazyExpireMap<K, V> where K: Hash + Eq {
    map: DashMap<K, (V, Instant)>,
}

impl<K, V> Deref for LazyExpireMap<K, V>
where K: Hash + Eq {
    type Target = DashMap<K, (V, Instant)>;

    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<K: Hash + Eq, V> Default for LazyExpireMap<K, V> {
    fn default() -> Self {
        Self { map: Default::default() }
    }
}

impl<K: Hash + Eq, V> LazyExpireMap<K, V> {
    pub fn get(&self, key: &K) -> Option<impl Deref<Target = V> + '_> {
        self.map.get(key)?.try_map(|v| if v.1 > Instant::now() {
            Some(&v.0)
        } else {
            None
        }).ok()
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        self.map.remove(key).map(|v| v.1.0)
    }

    pub fn insert_for(&self, duration: Duration, key: K, value: V) -> Option<V> {
        self.insert_until(Instant::now() + duration, key, value)
    }

    pub fn insert_until(&self, instant: impl Into<Instant>, key: K, value: V) -> Option<V> {
        self.map.insert(key, (value, instant.into())).map(|(v, _)| v)
    }

    pub fn retain_expired(&self) {
        self.map.retain(|_, v| v.1 < Instant::now())
    }
}
