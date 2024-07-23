use std::any::Any;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use crate::rocksdb_backend::RocksDBBackend;
use crate::runtime_env::RuntimeEnv;

/// A trait that provides a way to downcast to `RuntimeEnv`
pub trait AsRuntimeEnv: Debug {
    /// Returns self as a reference to `RuntimeEnv`, if possible.
    fn as_runtime_env(&self) -> Option<&RuntimeEnv>;
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Denormalized RuntimeEnv that can be downcasted to `RuntimeEnv`
#[derive(Clone)]
pub struct DenormalizedRuntimeEnv {
    pub runtime_env: Arc<dyn AsRuntimeEnv + Send + Sync>,
    pub rocksdb: Arc<RocksDBBackend>, // Added rocksdb field
}

impl Debug for DenormalizedRuntimeEnv {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "DenormalizedRuntimeEnv {{ ... }}") // Simplified for brevity
    }
}

impl DenormalizedRuntimeEnv {
    /// Create a new DenormalizedRuntimeEnv
    pub fn new(runtime_env: RuntimeEnv, rocksdb: Arc<RocksDBBackend>) -> Self {
        Self {
            runtime_env: Arc::new(runtime_env),
            rocksdb,
        }
    }

    pub fn rocksdb(&self) -> Arc<RocksDBBackend> {
        self.rocksdb.clone()
    }

    /// Downcast to `RuntimeEnv`
    pub fn runtime_env(&self) -> &RuntimeEnv {
        self.runtime_env.as_runtime_env().unwrap()
    }
}

impl AsRuntimeEnv for RuntimeEnv {
    fn as_runtime_env(&self) -> Option<&RuntimeEnv> {
        Some(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl AsRuntimeEnv for DenormalizedRuntimeEnv {
    fn as_runtime_env(&self) -> Option<&RuntimeEnv> {
        self.runtime_env.as_runtime_env()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
pub trait ExtendedEnv {
    fn as_any(&self) -> &dyn Any;
    fn some_method(&self);
}

// Implement the trait for RuntimeEnv
impl ExtendedEnv for RuntimeEnv {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn some_method(&self) {
        println!("Called some_method on RuntimeEnv");
    }
}

// Create your own extended runtime environment
pub struct MyRuntimeEnv {
    pub env: Arc<RuntimeEnv>,
    // Add additional fields or methods here
}

impl MyRuntimeEnv {
    pub fn new(env: Arc<RuntimeEnv>) -> Self {
        MyRuntimeEnv { env }
    }

    pub fn my_method(&self) {
        println!("MyRuntimeEnv specific method");
    }
}

impl ExtendedEnv for MyRuntimeEnv {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn some_method(&self) {
        self.env.some_method();
        println!("Called some_method on MyRuntimeEnv");
    }
}
