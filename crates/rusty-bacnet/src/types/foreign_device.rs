use super::*;

use bacnet_runtime::{ForeignDeviceRegistrationState, ForeignDeviceRegistrationStatus};

/// Immutable snapshot of a BACnet/IP foreign-device registration.
#[pyclass(name = "ForeignDeviceStatus", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyForeignDeviceStatus {
    #[pyo3(get)]
    state: String,
    #[pyo3(get)]
    last_result: Option<u16>,
    #[pyo3(get)]
    seconds_to_renewal: Option<u64>,
}

impl PyForeignDeviceStatus {
    pub(crate) fn from_rust(status: ForeignDeviceRegistrationStatus) -> Self {
        let (state, seconds_to_renewal) = match status.state {
            ForeignDeviceRegistrationState::Pending => ("pending", status.seconds_to_renewal),
            ForeignDeviceRegistrationState::Registered => ("registered", status.seconds_to_renewal),
            ForeignDeviceRegistrationState::Rejected => ("rejected", None),
            ForeignDeviceRegistrationState::Expired => ("expired", None),
        };
        Self {
            state: state.to_owned(),
            last_result: status.last_result_code.map(|code| code.to_raw()),
            seconds_to_renewal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::BvlcResultCode;

    fn status(state: ForeignDeviceRegistrationState) -> ForeignDeviceRegistrationStatus {
        ForeignDeviceRegistrationStatus {
            state,
            last_result_code: Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK),
            seconds_to_renewal: Some(17),
        }
    }

    #[test]
    fn python_status_projects_all_lifecycle_states_and_result_codes() {
        for (state, expected, retains_renewal) in [
            (ForeignDeviceRegistrationState::Pending, "pending", true),
            (
                ForeignDeviceRegistrationState::Registered,
                "registered",
                true,
            ),
            (ForeignDeviceRegistrationState::Rejected, "rejected", false),
            (ForeignDeviceRegistrationState::Expired, "expired", false),
        ] {
            let projected = PyForeignDeviceStatus::from_rust(status(state));
            assert_eq!(projected.state, expected);
            assert_eq!(
                projected.last_result,
                Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK.to_raw())
            );
            assert_eq!(projected.seconds_to_renewal, retains_renewal.then_some(17));
        }
    }
}
