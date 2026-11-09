//! Küçük, bağımlılıksız LRU. Kapasite küçük (yüzler) olduğu için çıkarma doğrusal tarama.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;

pub struct Lru<K, V> {
    map: HashMap<K, (V, u64)>,
    tick: u64,
    cap: usize,
}

impl<K: Hash + Eq, V> Lru<K, V> {
    pub fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            tick: 0,
            cap: cap.max(1),
        }
    }

    /// Değeri döner ve en yeni kullanılan olarak işaretler.
    pub fn get<Q>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(key).map(|(v, t)| {
            *t = tick;
            &*v
        })
    }

    pub fn contains<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.contains_key(key)
    }

    /// Ekler; kapasite doluysa en uzun süredir kullanılmayanı çıkarır.
    pub fn insert(&mut self, key: K, value: V) {
        self.tick += 1;
        if self.map.len() >= self.cap
            && !self.map.contains_key(&key)
            && let Some(min) = self.map.values().map(|(_, t)| *t).min()
        {
            // Tick'ler benzersiz: tam olarak en eski girdi çıkar.
            self.map.retain(|_, (_, t)| *t != min);
        }
        self.map.insert(key, (value, self.tick));
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used() {
        let mut lru = Lru::new(2);
        lru.insert("a", 1);
        lru.insert("b", 2);
        assert_eq!(lru.get("a"), Some(&1));
        lru.insert("c", 3);
        assert!(lru.contains("a") && lru.contains("c"));
        assert!(!lru.contains("b"));
        assert_eq!(lru.len(), 2);
    }

    #[test]
    fn replacing_key_does_not_evict() {
        let mut lru = Lru::new(2);
        lru.insert("a", 1);
        lru.insert("b", 2);
        lru.insert("a", 3);
        assert_eq!(lru.get("a"), Some(&3));
        assert!(lru.contains("b"));
    }
}
