use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::TryLockError::WouldBlock;

use tokio::sync::OwnedMutexGuard as Omg;

use crate::{AppError, AppResult};

/// Map of Mutexes
pub struct MutexMap<Key, Val> {
    map: Arc<std::sync::Mutex<HashMap<Key, Arc<tokio::sync::Mutex<Val>>>>>,
}

pub struct MutexMapGuard<Key, Val> {
    map: Arc<std::sync::Mutex<HashMap<Key, Arc<tokio::sync::Mutex<Val>>>>>,
    val: Omg<Val>,
}

impl<Key, Val> MutexMap<Key, Val>
where
    Key: Clone + Eq + Hash + Send,
    Val: Default + Send,
{
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: Default::default(),
        }
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn lock<'a, K>(&'a self, k: &'a K) -> MutexMapGuard<Key, Val>
    where
        K: Debug + Send + ?Sized + Sync,
        Key: TryFrom<&'a K>,
        <Key as TryFrom<&'a K>>::Error: Debug,
    {
        let val = self
            .map
            .lock()
            .expect("locked")
            .entry(k.try_into().expect("failed to construct key"))
            .or_default()
            .clone();

        MutexMapGuard::<Key, Val> {
            map: Arc::clone(&self.map),
            val: val.lock_owned().await,
        }
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub fn try_lock<'a, K>(&self, k: &'a K) -> AppResult<MutexMapGuard<Key, Val>>
    where
        K: Debug + Send + ?Sized + Sync,
        Key: TryFrom<&'a K>,
        <Key as TryFrom<&'a K>>::Error: Debug,
    {
        let val = self
            .map
            .lock()
            .expect("locked")
            .entry(k.try_into().expect("failed to construct key"))
            .or_default()
            .clone();

        Ok(MutexMapGuard::<Key, Val> {
            map: Arc::clone(&self.map),
            val: val
                .try_lock_owned()
                .map_err(|_| AppError::internal("would yield"))?,
        })
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub fn try_try_lock<'a, K>(&self, k: &'a K) -> AppResult<MutexMapGuard<Key, Val>>
    where
        K: Debug + Send + ?Sized + Sync,
        Key: TryFrom<&'a K>,
        <Key as TryFrom<&'a K>>::Error: Debug,
    {
        let val = self
            .map
            .try_lock()
            .map_err(|e| match e {
                WouldBlock => AppError::internal("would block"),
                _ => panic!("{e:?}"),
            })?
            .entry(k.try_into().expect("failed to construct key"))
            .or_default()
            .clone();

        Ok(MutexMapGuard::<Key, Val> {
            map: Arc::clone(&self.map),
            val: val
                .try_lock_owned()
                .map_err(|_| AppError::internal("would yield"))?,
        })
    }

    #[must_use]
    pub fn contains(&self, k: &Key) -> bool {
        self.map.lock().expect("locked").contains_key(k)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.lock().expect("locked").is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.map.lock().expect("locked").len()
    }
}

impl<Key, Val> Default for MutexMap<Key, Val>
where
    Key: Clone + Eq + Hash + Send,
    Val: Default + Send,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<Key, Val> Drop for MutexMapGuard<Key, Val> {
    #[tracing::instrument(name = "unlock", level = "trace", skip_all)]
    fn drop(&mut self) {
        if Arc::strong_count(Omg::mutex(&self.val)) <= 2 {
            self.map.lock().expect("locked").retain(|_, val| {
                !Arc::ptr_eq(val, Omg::mutex(&self.val)) || Arc::strong_count(val) > 2
            });
        }
    }
}
