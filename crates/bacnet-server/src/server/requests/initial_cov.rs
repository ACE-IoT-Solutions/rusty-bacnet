use super::*;

pub(super) enum InitialCovNotification {
    Single(CovSubscription),
    Multiple(Vec<CovSubscription>),
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fire_initial_cov_notifications(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        notification_transactions: &Arc<NotificationTransactions>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        notifications: &[InitialCovNotification],
    ) {
        for notification in notifications {
            match notification {
                InitialCovNotification::Single(subscription) => {
                    Self::fire_initial_cov_notification(
                        db,
                        network,
                        cov_table,
                        cov_in_flight,
                        notification_transactions,
                        comm_state,
                        config,
                        subscription,
                    )
                    .await;
                }
                InitialCovNotification::Multiple(subscriptions) => {
                    Self::fire_initial_cov_notification_multiple(
                        db,
                        network,
                        cov_table,
                        cov_in_flight,
                        notification_transactions,
                        comm_state,
                        config,
                        subscriptions,
                    )
                    .await;
                }
            }
        }
    }
}
