use std::{
    future::{Future, IntoFuture},
    io,
    net::SocketAddr,
};

use axum::Router;
use tokio::{net::TcpListener, sync::watch};

pub async fn bind(
    business: SocketAddr,
    health: SocketAddr,
) -> io::Result<(TcpListener, TcpListener)> {
    let business = TcpListener::bind(business).await?;
    // If the second bind fails, dropping the first listener releases its port.
    let health = TcpListener::bind(health).await?;
    Ok((business, health))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::{PrimaryAppState, primary_health_router, primary_router},
        auth::Authenticator,
    };
    use mem0_adapter::{Mem0Adapter, Mem0Config};
    use std::{sync::Arc, time::Duration};

    #[tokio::test]
    async fn occupied_health_port_releases_business_port() {
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let available = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let business = available.local_addr().unwrap();
        drop(available);
        assert!(
            bind(business, occupied.local_addr().unwrap())
                .await
                .is_err()
        );
        assert!(TcpListener::bind(business).await.is_ok());
    }

    #[tokio::test]
    async fn isolated_routes_preserve_authentication_and_shutdown_releases_both_ports() {
        let listeners = bind(
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
        let business = listeners.0.local_addr().unwrap();
        let health = listeners.1.local_addr().unwrap();
        let fixture = uuid::Uuid::new_v4().simple().to_string();
        let state = Arc::new(PrimaryAppState {
            mem0: Arc::new(
                Mem0Adapter::new(Mem0Config {
                    base_url: format!("http://{health}"),
                    api_key: Some(fixture.clone()),
                    timeout: Duration::from_secs(1),
                })
                .unwrap(),
            ),
            authenticator: Authenticator::trusted_dynamic_tenant(),
        });
        let (stop, shutdown) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(serve(
            listeners,
            primary_router(Arc::clone(&state)),
            primary_health_router(state),
            async {
                let _ = shutdown.await;
            },
        ));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        for path in ["/health/live", "/health/ready"] {
            assert_eq!(
                client
                    .get(format!("http://{business}{path}"))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                404
            );
        }
        assert_eq!(
            client
                .get(format!("http://{health}/health/live"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            client
                .delete(format!("http://{health}/v1/runtime/memory/unused"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            client
                .delete(format!("http://{business}/v1/runtime/memory/unused"))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        drop(client);
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(TcpListener::bind(business).await.is_ok());
        assert!(TcpListener::bind(health).await.is_ok());
    }
}

pub async fn serve(
    listeners: (TcpListener, TcpListener),
    business: Router,
    health: Router,
    shutdown: impl Future<Output = ()>,
) -> io::Result<()> {
    let (stop, mut business_stop) = watch::channel(());
    let mut health_stop = business_stop.clone();
    let business = axum::serve(listeners.0, business)
        .with_graceful_shutdown(async move {
            let _ = business_stop.changed().await;
        })
        .into_future();
    let health = axum::serve(listeners.1, health)
        .with_graceful_shutdown(async move {
            let _ = health_stop.changed().await;
        })
        .into_future();
    tokio::pin!(business, health);
    tokio::select! {
        result = &mut business => { drop(stop); health.await?; result },
        result = &mut health => { drop(stop); business.await?; result },
        _ = shutdown => {
            drop(stop);
            tokio::try_join!(business, health)?;
            Ok(())
        },
    }
}
