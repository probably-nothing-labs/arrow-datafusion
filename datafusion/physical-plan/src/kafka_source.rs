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

use std::collections::HashMap;

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use arrow::datatypes::TimestampMillisecondType;
use arrow_array::{
    Array, ArrayRef, PrimitiveArray, RecordBatch, StringArray, StructArray,
};
use arrow_schema::{DataType, Field, SchemaRef, TimeUnit};
use datafusion_common::franz_arrow::json_records_to_arrow_record_batch;
use datafusion_execution::denormalized_runtime_env::{
    DenormalizedRuntimeEnv, RuntimeEnvExt,
};
use datafusion_execution::rocksdb_backend::RocksDBBackend;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, error};

use crate::stream::RecordBatchReceiverStreamBuilder;
use crate::streaming::PartitionStream;
use arrow::compute::{max, min};
use datafusion_common::{plan_err, DataFusionError};
use datafusion_execution::{SendableRecordBatchStream, TaskContext};
use datafusion_expr::Expr;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::{ClientConfig, Message, Timestamp, TopicPartitionList};
// Import min_max function

use crate::time::{array_to_timestamp_array, TimestampUnit};

/// The data encoding for [`StreamTable`]
#[derive(Debug, Clone)]
pub enum StreamEncoding {
    Avro,
    Json,
}

impl FromStr for StreamEncoding {
    type Err = DataFusionError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "avro" => Ok(Self::Avro),
            "json" => Ok(Self::Json),
            _ => plan_err!("Unrecognised StreamEncoding {}", s),
        }
    }
}

/// The configuration for a [`StreamTable`]
#[derive(Debug)]
pub struct KafkaStreamConfig {
    pub original_schema: SchemaRef,
    pub schema: SchemaRef,
    pub topic: String,
    pub batch_size: usize,
    pub encoding: StreamEncoding,
    pub order: Vec<Vec<Expr>>,
    pub partitions: i32,
    pub timestamp_column: String,
    pub timestamp_unit: TimestampUnit,
    pub bootstrap_servers: String,
    pub consumer_group_id: String,
    pub offset_reset: String,
    //constraints: Constraints,
}

impl KafkaStreamConfig {
    /// Specify the batch size
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Specify an encoding for the stream
    pub fn with_encoding(mut self, encoding: StreamEncoding) -> Self {
        self.encoding = encoding;
        self
    }
}

pub struct KafkaStreamRead {
    pub config: Arc<KafkaStreamConfig>,
    pub assigned_partitions: Vec<i32>,
}

#[derive(Serialize, Deserialize)]
struct BatchReadMetadata {
    epoch: i32,
    min_timestamp: Option<i64>,
    max_timestamp: Option<i64>,
    offsets_read: Vec<(i32, i64)>,
}

impl BatchReadMetadata {
    // Serialize to Vec<u8> using bincode
    fn to_bytes(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    // Deserialize from Vec<u8> using bincode
    fn from_bytes(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }
}

impl PartitionStream for KafkaStreamRead {
    fn schema(&self) -> &SchemaRef {
        &self.config.schema
    }

    fn execute(&self, ctx: Arc<TaskContext>) -> SendableRecordBatchStream {
        let mut assigned_partitions = TopicPartitionList::new();
        let topic = self.config.topic.clone();
        for partition in self.assigned_partitions.clone() {
            assigned_partitions.add_partition(self.config.topic.as_str(), partition);
        }
        let partition_tag = self
            .assigned_partitions
            .iter()
            .map(|&x| x.to_string())
            .collect::<Vec<String>>()
            .join("_");

        let mut client_config = ClientConfig::new();

        // RuntimeEnv lives for the entire lifetime of the pipeline.
        // Needed an end run around borrow checker: error[E0716]: temporary value dropped while borrowed
        let state_backend = unsafe {
            std::mem::transmute::<&Arc<RocksDBBackend>, &'static Arc<RocksDBBackend>>(
                ctx.runtime_env()
                    .as_any()
                    .downcast_ref::<DenormalizedRuntimeEnv>()
                    .expect("Expected DenormalizedRuntimeEnv")
                    .rocksdb(),
            )
        };

        client_config
            .set(
                "bootstrap.servers",
                self.config.bootstrap_servers.to_string(),
            )
            .set("enable.auto.commit", "false") // Disable auto-commit for manual offset control
            // @TODO we need to store offsets somehow
            .set("auto.offset.reset", self.config.offset_reset.to_string())
            .set("group.id", self.config.consumer_group_id.to_string());

        let consumer: StreamConsumer =
            client_config.create().expect("Consumer creation failed");

        consumer
            .assign(&assigned_partitions)
            .expect("Partition assignment failed.");

        //let schema = self.config.schema.clone();

        let mut builder =
            RecordBatchReceiverStreamBuilder::new(self.config.schema.clone(), 1);
        let tx = builder.tx();
        let canonical_schema = self.config.schema.clone();
        let json_schema = self.config.original_schema.clone();
        let timestamp_column: String = self.config.timestamp_column.clone();
        let timestamp_unit = self.config.timestamp_unit.clone();

        let state_namespace = format!("kafka_source_{}", topic);
        let _ = state_backend.create_cf(state_namespace.as_str());
        let _ = builder.spawn(async move {
            let mut epoch = 0;
            loop {
                let mut offsets_read: Vec<(i32, i64)> = vec![];
                let batch: Vec<serde_json::Value> = consumer
                    .stream()
                    .take_until(tokio::time::sleep(Duration::from_secs(1)))
                    .map(|message| match message {
                        Ok(m) => {
                            let timestamp = match m.timestamp() {
                                Timestamp::NotAvailable => -1_i64,
                                Timestamp::CreateTime(ts) => ts,
                                Timestamp::LogAppendTime(ts) => ts,
                            };
                            let key = m.key();

                            let payload = m.payload().expect("Message payload is empty");
                            let mut deserialized_record: HashMap<String, Value> =
                                serde_json::from_slice(payload).unwrap();
                            deserialized_record.insert(
                                "kafka_timestamp".to_string(),
                                Value::from(timestamp),
                            );
                            if let Some(key) = key {
                                deserialized_record.insert(
                                    "kafka_key".to_string(),
                                    Value::from(String::from_utf8_lossy(key)),
                                );
                            } else {
                                deserialized_record.insert(
                                    "kafka_key".to_string(),
                                    Value::from(String::from("")),
                                );
                            }
                            let new_payload =
                                serde_json::to_value(deserialized_record).unwrap();
                            offsets_read.push((m.partition(), m.offset()));
                            new_payload
                        }
                        Err(err) => {
                            error!("Error reading from Kafka {:?}", err);
                            panic!("Error reading from Kafka {:?}", err)
                        }
                    })
                    .collect()
                    .await;

                debug!("Batch size {}", batch.len());

                let record_batch: RecordBatch =
                    json_records_to_arrow_record_batch(batch, json_schema.clone());

                let ts_column = record_batch
                    .column_by_name(timestamp_column.as_str())
                    .map(|ts_col| {
                        Arc::new(array_to_timestamp_array(ts_col, timestamp_unit.clone()))
                    })
                    .unwrap();

                let binary_vec = Vec::from_iter(
                    std::iter::repeat(String::from("no_barrier")).take(ts_column.len()),
                );
                let barrier_batch_column = StringArray::from(binary_vec);

                let ts_array = ts_column
                    .as_any()
                    .downcast_ref::<PrimitiveArray<TimestampMillisecondType>>()
                    .unwrap();

                let max_timestamp: Option<_> = max::<TimestampMillisecondType>(&ts_array);
                let min_timestamp: Option<_> = min::<TimestampMillisecondType>(&ts_array);
                debug!("min: {:?}, max: {:?}", min_timestamp, max_timestamp);
                let mut columns: Vec<Arc<dyn Array>> = record_batch.columns().to_vec();

                let metadata_column = StructArray::from(vec![
                    (
                        Arc::new(Field::new("barrier_batch", DataType::Utf8, false)),
                        Arc::new(barrier_batch_column) as ArrayRef,
                    ),
                    (
                        Arc::new(Field::new(
                            "canonical_timestamp",
                            DataType::Timestamp(TimeUnit::Millisecond, None),
                            true,
                        )),
                        ts_column as ArrayRef,
                    ),
                ]);
                columns.push(Arc::new(metadata_column));

                let timestamped_record_batch: RecordBatch =
                    RecordBatch::try_new(canonical_schema.clone(), columns).unwrap();
                let tx_result = tx.send(Ok(timestamped_record_batch)).await;
                match tx_result {
                    Ok(m) => {
                        let _ = state_backend
                            .put_state(
                                &state_namespace,
                                partition_tag.clone().into_bytes(),
                                BatchReadMetadata {
                                    epoch,
                                    min_timestamp,
                                    max_timestamp,
                                    offsets_read,
                                }
                                .to_bytes()
                                .unwrap(), //TODO: Fix the error threading.
                            )
                            .await;
                    }
                    Err(err) => error!("result err {:?}", err),
                }
                epoch += 1;
            }
        });
        builder.build()
    }
}
