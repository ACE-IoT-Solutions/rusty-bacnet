//! Projection of terminal TSM responses into the public error model.

use super::*;

pub(crate) fn confirmed_response_result(response: TsmResponse) -> Result<Bytes, Error> {
    match response {
        TsmResponse::SimpleAck => Ok(Bytes::new()),
        TsmResponse::ComplexAck { service_data } => Ok(service_data),
        TsmResponse::Error {
            class,
            code,
            detail,
        } => Err(Error::protocol(class, code, detail)),
        TsmResponse::Reject { reason } => Err(Error::Reject { reason }),
        TsmResponse::Abort { reason } => Err(Error::Abort { reason }),
        TsmResponse::NetworkPathTooLong { dnet } => Err(Error::RoutedPathTooLong { dnet }),
    }
}
