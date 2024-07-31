// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::{env, sync::OnceLock};

use datafusion_common::DataFusionError;
use log::debug;
use rocksdb::{
    BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options,
    DB,
};
use std::sync::Arc;

pub struct RocksDBBackend {
    db: DBWithThreadMode<MultiThreaded>,
}

impl RocksDBBackend {
    pub fn new(path: &str) -> Result<Self, DataFusionError> {
        let dir = env::temp_dir();
        let db_path = format!("{}{}", dir.display(), path);
        debug!("Opening rocksdb at {}", db_path);

        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);

        // List all column families in the existing database
        let cf_names = DB::list_cf(&db_opts, &db_path).map_err(|e| {
            DataFusionError::Internal(format!("Failed to list column families: {}", e))
        })?;

        if cf_names.is_empty() {
            // If no column families, open the DB normally
            let db = DBWithThreadMode::<MultiThreaded>::open(&db_opts, &db_path)
                .map_err(|e| {
                    DataFusionError::Internal(format!("Failed to open RocksDB: {}", e))
                })?;
            Ok(RocksDBBackend { db })
        } else {
            // If column families exist, open the DB with all existing column families
            let cf_descriptors: Vec<ColumnFamilyDescriptor> = cf_names
                .iter()
                .map(|name| ColumnFamilyDescriptor::new(name, Options::default()))
                .collect();

            let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
                &db_opts,
                &db_path,
                cf_descriptors,
            )
            .map_err(|e| {
                DataFusionError::Internal(format!(
                    "Failed to open RocksDB with column families: {}",
                    e
                ))
            })?;
            Ok(RocksDBBackend { db })
        }
    }

    pub fn create_cf(&self, namespace: &str) -> Result<(), DataFusionError> {
        let cf_opts: Options = Options::default();
        DBWithThreadMode::<MultiThreaded>::create_cf(&self.db, namespace, &cf_opts)
            .map_err(|e| DataFusionError::Internal(e.into_string()))?;
        //self.namespaces.insert(namespace.to_string());
        Ok(())
    }

    pub fn get_cf(
        &self,
        namespace: &str,
    ) -> Result<Arc<BoundColumnFamily>, DataFusionError> {
        self.db.cf_handle(namespace).ok_or_else(|| {
            DataFusionError::Internal("namespace does not exist.".to_string())
        })
    }

    fn namespaced_key(&self, namespace: &str, key: &[u8]) -> Vec<u8> {
        let mut nk: Vec<u8> = namespace.as_bytes().to_vec();
        nk.push(b':');
        nk.extend_from_slice(key);
        nk
    }

    #[warn(dead_code)]
    pub(crate) fn destroy(&self) -> Result<(), DataFusionError> {
        let ret = DB::destroy(&Options::default(), self.db.path());
        Ok(())
    }

    pub async fn put_state(
        &self,
        namespace: &str,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<(), DataFusionError> {
        /*         if !self.namespaces.contains(namespace) {
            return Err(StateBackendError {
                message: "Namespace does not exist.".into(),
            });
        } */
        let cf: Arc<BoundColumnFamily> = self.get_cf(namespace)?;
        let namespaced_key: Vec<u8> = self.namespaced_key(namespace, &key);
        self.db
            .put_cf(&cf, namespaced_key, value)
            .map_err(|e| DataFusionError::Internal(e.to_string()))
    }

    pub async fn get_state(
        &self,
        namespace: &str,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, DataFusionError> {
        let cf: Arc<BoundColumnFamily> = self.get_cf(namespace)?;
        let namespaced_key: Vec<u8> = self.namespaced_key(namespace, &key);

        match self
            .db
            .get_cf(&cf, namespaced_key)
            .map_err(|e| DataFusionError::Internal(e.to_string()))?
        {
            Some(serialized_value) => Ok(Some(serialized_value)),
            None => Ok(None),
        }
    }

    pub async fn delete_state(
        &self,
        namespace: &str,
        key: Vec<u8>,
    ) -> Result<(), DataFusionError> {
        let cf: Arc<BoundColumnFamily> = self.get_cf(namespace)?;
        let namespaced_key: Vec<u8> = self.namespaced_key(namespace, &key);

        self.db
            .delete_cf(&cf, &namespaced_key)
            .map_err(|e| DataFusionError::Internal(e.to_string()))?;
        Ok(())
    }
}

static GLOBAL_ROCKSDB: OnceLock<Arc<RocksDBBackend>> = OnceLock::new();

pub fn initialize_global_rocksdb(path: &str) -> Result<(), DataFusionError> {
    let backend = RocksDBBackend::new(path)?;
    GLOBAL_ROCKSDB.set(Arc::new(backend)).map_err(|_| {
        DataFusionError::Internal("Global RocksDBBackend already initialized".to_string())
    })
}

pub fn get_global_rocksdb() -> Result<Arc<RocksDBBackend>, DataFusionError> {
    GLOBAL_ROCKSDB.get().cloned().ok_or_else(|| {
        DataFusionError::Internal("Global RocksDBBackend not initialized".to_string())
    })
}
