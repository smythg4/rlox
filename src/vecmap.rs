use fnv::FnvHashMap;
use std::hash::Hash;

const THRESHOLD: usize = 8;

#[derive(Debug)]
pub enum VecMap<K: Hash + Eq + Clone, V: Copy> {
    Small(Vec<(K, V)>),
    Large(FnvHashMap<K, V>),
}

impl<K: Hash + Eq + Clone, V: Copy> Default for VecMap<K, V> {
    fn default() -> Self {
        VecMap::Small(Vec::with_capacity(THRESHOLD/2))
    }
}

impl<K: Hash + Eq + Clone, V: Copy> VecMap<K, V> {
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self {
            VecMap::Small(vm) => {
                if let Some(entry) = vm.iter_mut().find(|(k, _)| k == &key) {
                    let old = entry.1;
                    entry.1 = value;
                    return Some(old);
                }
                vm.push((key, value));

                if vm.len() > THRESHOLD {
                    self.into_large();
                }
                None
            }
            VecMap::Large(vm) => vm.insert(key, value),
        }
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
      match self {
          VecMap::Small(vm) => {
              if let Some(pos) = vm.iter().position(|(k, _)| k == key) {
                  Some(vm.swap_remove(pos).1)
              } else {
                  None
              }
          }
          VecMap::Large(vm) => vm.remove(key),
      }
    }

  pub fn get<Q>(&self, key: &Q) -> Option<&V>
  // all this 'Q' ceremony allows us to send a &str instead of a &String
  where
      K: std::borrow::Borrow<Q>,
      Q: PartialEq + ?Sized + Hash + Eq,
  {
      match self {
          VecMap::Small(vm) => vm.iter().find(|(k, _)| k.borrow() == key).map(|(_, v)| v),
          VecMap::Large(vm) => vm.get(key),
      }
  }

  pub fn contains_key<Q>(&self, key: &Q) -> bool
  // all this 'Q' ceremony allows us to send a &str instead of a &String
  where
      K: std::borrow::Borrow<Q>,
      Q: PartialEq + ?Sized + Hash + Eq,
  {
      match self {
          VecMap::Small(vm) => vm.iter().any(|(k, _)| k.borrow() == key),
          VecMap::Large(vm) => vm.contains_key(key),
      }
  }

pub fn keys(&self) -> Box<dyn Iterator<Item = &K> + '_> {
      match self {
          VecMap::Small(vm) => Box::new(vm.iter().map(|(k, _)| k)),
          VecMap::Large(vm) => Box::new(vm.keys()),
      }
  }

    pub fn values(&self) -> Box<dyn Iterator<Item = &V> + '_> {
      match self {
          VecMap::Small(vm) => Box::new(vm.iter().map(|(_, v)| v)),
          VecMap::Large(vm) => Box::new(vm.values()),
      }
  }

    pub fn iter(&self) -> Box<dyn Iterator<Item = (&K, &V)> + '_> {
        match self {
            VecMap::Small(vm) => Box::new(vm.iter().map(|(k, v)| (k, v))),
            VecMap::Large(vm) => Box::new(vm.iter()),
        }
    }

    pub fn extend(&mut self, other: &VecMap<K, V>) {
        for (k, v) in other.iter() {
            self.insert(k.clone(), *v);
        }
    }

    fn into_large(&mut self) {
        let inner = match self {
            VecMap::Small(vm) => std::mem::take(vm),
            _ => return,
        };

        *self = VecMap::Large(inner.into_iter().collect());
    }
}
