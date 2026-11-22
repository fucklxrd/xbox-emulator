use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Package {
    pub flags: u32,
    pub install_time: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Family {
    pub dacl_sddl: String,
    pub packages: BTreeMap<String, Package>,
}

#[derive(Default)]
pub struct Store {
    pub families: BTreeMap<(String, String), Family>,
    pub removed: BTreeMap<(String, String, String), ()>,
}

static STORE: Mutex<Store> = Mutex::new(Store { families: BTreeMap::new(), removed: BTreeMap::new() });

pub fn store() -> MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

pub enum Lookup {
    Present(u64),
    Removed,
    Untouched,
}

fn fold(s: &str) -> String {
    s.to_lowercase()
}

impl Store {
    pub fn add(&mut self, user: &str, family: &str, full: &str, sddl: &str, flags: u32, install_time: u64) -> bool {
        let fkey = (fold(user), fold(family));
        let created = !self.families.contains_key(&fkey);
        let f = self.families.entry(fkey).or_default();
        if created {
            f.dacl_sddl = sddl.to_string();
        }
        let p = f.packages.entry(fold(full)).or_default();
        p.flags = flags;
        p.install_time = install_time;
        self.removed.remove(&(fold(user), fold(family), fold(full)));
        created
    }

    pub fn remove(&mut self, user: &str, family: &str, full: &str) {
        let fkey = (fold(user), fold(family));
        self.removed.insert((fold(user), fold(family), fold(full)), ());
        if let Some(f) = self.families.get_mut(&fkey) {
            f.packages.remove(&fold(full));
            if f.packages.is_empty() {
                self.families.remove(&fkey);
            }
        }
    }

    pub fn lookup(&self, user: &str, family: &str, full: &str) -> Lookup {
        if let Some(p) = self.families.get(&(fold(user), fold(family))).and_then(|f| f.packages.get(&fold(full))) {
            return Lookup::Present(p.install_time);
        }
        if self.removed.contains_key(&(fold(user), fold(family), fold(full))) {
            return Lookup::Removed;
        }
        Lookup::Untouched
    }

    pub fn family(&self, user: &str, family: &str) -> Option<Family> {
        self.families.get(&(fold(user), fold(family))).cloned()
    }
}
