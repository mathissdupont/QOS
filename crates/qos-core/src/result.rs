use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QResult {
    pub status: ResultStatus,          // OK / ERROR
    pub counts: BTreeMap<String, u64>, // boş olabilir
    pub meta: String,
    pub error: Option<String>,         // ERROR ise dolu
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResultStatus {
    Ok,
    Error,
}
