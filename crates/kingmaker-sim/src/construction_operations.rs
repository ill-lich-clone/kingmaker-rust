//! Private authority from closed investment/construction plans, never caller payloads.
use crate::operation_session::{OpResult, OperationSession};
use kingmaker_core::operations::{EconomicOperation, OperationKind};
pub(crate) fn construction_kind(kind: &OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::CapitalistInvestment
            | OperationKind::ConstructionStaffTransfer
            | OperationKind::ConstructionWait
            | OperationKind::ConstructionProgress
            | OperationKind::ConstructionMaterials
            | OperationKind::ConstructionContractFee
            | OperationKind::FactoryWorkingCapital
            | OperationKind::ProjectRefund
            | OperationKind::ConstructionCancelled
            | OperationKind::ConstructionPhaseBoundary
    )
}
pub(crate) fn stage_construction_operations(
    session: &mut OperationSession,
    operations: &[EconomicOperation],
) -> OpResult<()> {
    for op in operations {
        session.stage_construction(op)?;
    }
    Ok(())
}
