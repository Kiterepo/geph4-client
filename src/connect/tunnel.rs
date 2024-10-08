use bytes::Bytes;

use derivative::Derivative;
use geph5_broker_protocol::Credential;
use geph5_client::{BridgeMode, BrokerSource, Config};
use geph_nat::GephNat;
use parking_lot::RwLock;

use sillad::Pipe;
use smol::{
    channel::{Receiver, Sender},
    Task,
};
use smol_str::SmolStr;
use std::{
    net::SocketAddr,
    time::{Duration, SystemTime},
};
use stdcode::StdcodeSerializeExt;
use tmelcrypt::Hashable;

use sosistab2::Stream;
use std::sync::Arc;

use std::net::Ipv4Addr;

use crate::config::{ConnectOpt, GEPH5_CONFIG_TEMPLATE};

use super::stats::{gatherer::StatItem, STATS_GATHERER};

#[derive(Clone)]
pub struct BinderTunnelParams {
    pub exit_server: Option<String>,
    pub use_bridges: bool,
    pub force_bridge: Option<Ipv4Addr>,
    pub force_protocol: Option<String>,
}

#[derive(Clone)]
struct TunnelCtx {
    recv_socks5_conn: Receiver<(String, Sender<Stream>)>,

    connect_status: Arc<RwLock<ConnectionStatus>>,
    recv_vpn_outgoing: Receiver<Bytes>,
    send_vpn_incoming: Sender<Bytes>,
}

/// A ConnectionStatus shows the status of the tunnel.
#[derive(Clone, Derivative)]
#[derivative(Debug)]
pub enum ConnectionStatus {
    Connecting,
    Connected { protocol: SmolStr, address: SmolStr },
}

impl ConnectionStatus {
    pub fn connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }
}

/// A tunnel starts and keeps alive the best sosistab session it can under given constraints.
/// A sosistab Session is *a single end-to-end connection between a client and a server.*
/// This can be thought of as analogous to TcpStream, except all reads and writes are datagram-based and unreliable.
pub struct ClientTunnel {
    client: geph5_client::Client,
    _stat_reporter: Task<()>,
}

impl ClientTunnel {
    /// Creates a new ClientTunnel.
    pub fn new(opt: ConnectOpt) -> Self {
        let (username, password) = match &opt.auth.auth_kind {
            Some(crate::config::AuthKind::AuthPassword { username, password }) => {
                (username.clone(), password.clone())
            }
            _ => todo!(),
        };
        let mut config = GEPH5_CONFIG_TEMPLATE.clone();
        config.credentials = Credential::LegacyUsernamePassword { username, password };
        config.bridge_mode = if opt.use_bridges {
            BridgeMode::ForceBridges
        } else {
            BridgeMode::Auto
        };
        config.cache = Some(
            opt.auth
                .credential_cache
                .clone()
                .join(format!("cache-{}.db", opt.auth.stdcode().hash())),
        );
        log::debug!("cache path: {:?}", config.cache);
        let client = geph5_client::Client::start(config);
        let handle = client.control_client();
        let stat_reporter = smolscale::spawn(async move {
            loop {
                smol::Timer::after(Duration::from_secs(1)).await;
                let info = handle.conn_info().await.unwrap();
                let recv_bytes = handle.stat_num("total_rx_bytes".into()).await.unwrap();
                let send_bytes = handle.stat_num("total_tx_bytes".into()).await.unwrap();
                match info {
                    geph5_client::ConnInfo::Connecting => {}
                    geph5_client::ConnInfo::Connected(conn) => STATS_GATHERER.push(StatItem {
                        time: SystemTime::now(),
                        endpoint: conn.bridge.into(),
                        protocol: conn.protocol.into(),
                        ping: Duration::from_millis(100),
                        send_bytes: send_bytes as u64,
                        recv_bytes: recv_bytes as u64,
                    }),
                }
            }
        });
        Self {
            client,
            _stat_reporter: stat_reporter,
        }
    }

    /// Returns the current connection status.
    pub async fn status(&self) -> ConnectionStatus {
        let conn_info = self.client.control_client().conn_info().await.unwrap();
        match conn_info {
            geph5_client::ConnInfo::Connecting => ConnectionStatus::Connecting,
            geph5_client::ConnInfo::Connected(info) => ConnectionStatus::Connected {
                protocol: info.protocol.into(),
                address: info.bridge.into(),
            },
        }
    }

    /// Returns a sosistab stream to the given remote host.
    pub async fn connect_stream(&self, remote: &str) -> anyhow::Result<Box<dyn Pipe>> {
        self.client.open_conn(remote).await
    }

    pub async fn send_vpn(&self, msg: &[u8]) -> anyhow::Result<()> {
        self.client.send_vpn_packet(msg.to_vec().into()).await
    }

    pub async fn recv_vpn(&self) -> anyhow::Result<Bytes> {
        self.client.recv_vpn_packet().await
    }
}
