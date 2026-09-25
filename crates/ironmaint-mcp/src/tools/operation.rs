//! `operation.get` tool.

use ironmaint_core::OperationId;
use ironmaint_policy::PrivilegedOperation;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetInput {
    pub operation_id: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetOutput {
    pub operation: PrivilegedOperation,
}
