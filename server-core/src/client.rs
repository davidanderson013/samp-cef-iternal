use std::net::SocketAddr;
use std::time::Instant;

use network::PeerId;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum State {
    Connecting,
    Connected,
}

#[derive(Debug)]
pub struct Client {
    id: i32,
    state: State,
    addr: SocketAddr,
    peer: PeerId,
    bundle_pending_since: Option<Instant>,
}

impl Client {
    pub fn new(id: i32, peer: PeerId, addr: SocketAddr) -> Client {
        Client {
            id,
            addr,
            peer,
            state: State::Connecting,
            bundle_pending_since: None,
        }
    }

    /// Marks this client as waiting for an `ASSET_BUNDLE_READY` ack before
    /// the gamemode should be told it's safe to start creating browsers.
    pub fn start_bundle_pending(&mut self) {
        self.bundle_pending_since = Some(Instant::now());
    }

    pub fn clear_bundle_pending(&mut self) {
        self.bundle_pending_since = None;
    }

    pub fn bundle_pending(&self) -> bool {
        self.bundle_pending_since.is_some()
    }

    pub fn bundle_pending_since(&self) -> Option<Instant> {
        self.bundle_pending_since
    }

    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        self.state == State::Connected
    }

    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn set_state(&mut self, state: State) {
        self.state = state;
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn peer(&self) -> PeerId {
        self.peer
    }
}
