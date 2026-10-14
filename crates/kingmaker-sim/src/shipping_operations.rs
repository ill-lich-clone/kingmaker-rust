//! Sealed transport authority: only native phase planners can stage these exact operations.
use crate::operation_session::{OpResult, OperationSession};
use kingmaker_core::operations::{EconomicOperation, OperationKind};
pub(crate) fn shipping_kind(kind: &OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::ShipmentArrival
            | OperationKind::ShipmentDispatch
            | OperationKind::ShipmentInterrupted
            | OperationKind::ShipmentRecovery
            | OperationKind::ShipmentReturnDispatch
            | OperationKind::ShipmentReturnArrival
            | OperationKind::SupplyPhaseBoundary
            | OperationKind::StateProcurementPeriod
    )
}
pub(crate) fn stage_shipping_operations(
    session: &mut OperationSession,
    operations: &[EconomicOperation],
) -> OpResult<()> {
    for op in operations {
        session.stage_shipping(op)?;
    }
    Ok(())
}
