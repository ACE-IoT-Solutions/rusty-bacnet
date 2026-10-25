//! GetAlarmSummary and GetEnrollmentSummary, sent and decoded (#2077).

use super::*;

use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// GetAlarmSummary to a device on this network, its answer decoded.
    pub async fn get_alarm_summary(
        &self,
        destination_mac: &[u8],
    ) -> Result<GetAlarmSummaryAck, Error> {
        let ack = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::GET_ALARM_SUMMARY,
                &[],
            )
            .await?;
        GetAlarmSummaryAck::decode(&ack)
    }

    /// [`Self::get_alarm_summary`] to a discovered device, routed when it
    /// is behind a router. A device not in the table fails before sending.
    pub async fn get_alarm_summary_from_device(
        &self,
        device_instance: u32,
    ) -> Result<GetAlarmSummaryAck, Error> {
        let ack = self
            .confirmed_request_to_device(
                device_instance,
                ConfirmedServiceChoice::GET_ALARM_SUMMARY,
                &[],
            )
            .await?;
        GetAlarmSummaryAck::decode(&ack)
    }

    /// GetEnrollmentSummary to a device on this network, its answer
    /// decoded. A filter out of its range fails before sending.
    pub async fn get_enrollment_summary(
        &self,
        destination_mac: &[u8],
        request: &GetEnrollmentSummaryRequest,
    ) -> Result<GetEnrollmentSummaryAck, Error> {
        let mut buf = BytesMut::new();
        request.try_encode(&mut buf)?;
        let ack = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
                &buf,
            )
            .await?;
        GetEnrollmentSummaryAck::decode(&ack)
    }

    /// [`Self::get_enrollment_summary`] to a discovered device, routed when
    /// it is behind a router. A device not in the table fails before
    /// sending.
    pub async fn get_enrollment_summary_from_device(
        &self,
        device_instance: u32,
        request: &GetEnrollmentSummaryRequest,
    ) -> Result<GetEnrollmentSummaryAck, Error> {
        let mut buf = BytesMut::new();
        request.try_encode(&mut buf)?;
        let ack = self
            .confirmed_request_to_device(
                device_instance,
                ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
                &buf,
            )
            .await?;
        GetEnrollmentSummaryAck::decode(&ack)
    }

    /// A confirmed request to a discovered device: through its router when
    /// the table holds one, and otherwise to its MAC.
    async fn confirmed_request_to_device(
        &self,
        device_instance: u32,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        match routing {
            Some((network, address)) => {
                self.confirmed_request_routed(&mac, network, &address, service_choice, service_data)
                    .await
            }
            None => {
                self.confirmed_request(&mac, service_choice, service_data)
                    .await
            }
        }
    }
}
