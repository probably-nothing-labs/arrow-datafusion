use std::{any::Any, sync::Arc};

use datafusion_common::{DataFusionError, Result};

use crate::{
    rocksdb_backend::RocksDBBackend,
    runtime_env::{RuntimeConfig, RuntimeEnv},
};

pub struct DenormalizedRuntimeEnv {
    // Inherit all fields from RuntimeEnv
    runtime_env: RuntimeEnv,
    // Add RocksDB connection
    rocksdb: Arc<RocksDBBackend>,
}

impl DenormalizedRuntimeEnv {
    pub fn new(
        config: RuntimeConfig,
        rocksdb: Arc<RocksDBBackend>,
    ) -> Result<Self, DataFusionError> {
        let runtime_env = RuntimeEnv::new(config)?;
        Ok(Self {
            runtime_env,
            rocksdb,
        })
    }

    // Getter for RocksDB connection
    pub fn rocksdb(&self) -> &Arc<RocksDBBackend> {
        &self.rocksdb
    }
}

// Implement Deref to allow transparent access to RuntimeEnv methods
impl std::ops::Deref for DenormalizedRuntimeEnv {
    type Target = RuntimeEnv;

    fn deref(&self) -> &Self::Target {
        &self.runtime_env
    }
}

// Implement DerefMut if you need mutable access to RuntimeEnv fields
impl std::ops::DerefMut for DenormalizedRuntimeEnv {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.runtime_env
    }
}

// Implement From<DenormalizedRuntimeEnv> for RuntimeEnv
impl From<DenormalizedRuntimeEnv> for RuntimeEnv {
    fn from(env: DenormalizedRuntimeEnv) -> Self {
        env.runtime_env
    }
}

pub trait RuntimeEnvExt {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// Implement RuntimeEnvExt for RuntimeEnv
impl RuntimeEnvExt for RuntimeEnv {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
