//! Kept crate-private: public operation payloads cannot choose their own income authority.
use crate::operation_session::{OpResult, OperationSession};
use kingmaker_core::operations::EconomicOperation;
pub(crate) fn stage_income_operations(
    session: &mut OperationSession,
    operations: &[EconomicOperation],
) -> OpResult<()> {
    for op in operations {
        session.stage_income(op)?;
    }
    Ok(())
}
