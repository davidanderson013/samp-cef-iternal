use crossbeam_channel::{Receiver, Sender};
use messages::{packets, try_into_packet};
use network::{CertStrategy, Event as SocketEvent, PeerId, Socket};
use quick_protobuf::deserialize_from_slice;
use tracing::trace;

use std::collections::HashMap;
use std::convert::TryInto;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::AssetBundleInfo;
use crate::ServerEvent;
use crate::client::Client;
use crate::client::State;

/// Chunk size used when streaming `ui.bin` to a client over `AssetChunk`
/// packets. Small enough to keep each protobuf message modest, large
/// enough that a ~5 MB UI bundle is a few hundred messages, not thousands.
const ASSET_CHUNK_SIZE: usize = 32 * 1024;

/// How long the server waits for ASSET_BUNDLE_READY before giving up and
/// letting the gamemode proceed anyway. See `Server::check_bundle_timeouts`.
const ASSET_BUNDLE_TIMEOUT: Duration = Duration::from_secs(60);

enum Packet {
    Normal { peer: PeerId, bytes: Vec<u8> },
    Disconnect(PeerId),
}

impl Packet {
    fn new(peer: PeerId, bytes: Vec<u8>) -> Packet {
        Packet::Normal { peer, bytes }
    }

    fn disconnect(peer: PeerId) -> Packet {
        Packet::Disconnect(peer)
    }
}

pub struct Server {
    event_tx: Sender<ServerEvent>,
    event_rx: Receiver<ServerEvent>,
    sender: Sender<Packet>,
    allowed: HashMap<IpAddr, i32>,
    clients: HashMap<PeerId, Client>,
    players: HashMap<i32, PeerId>,
    asset_bundle: Option<Arc<AssetBundleInfo>>,
}

impl Server {
    pub fn new(addr: SocketAddr) -> Result<Arc<Mutex<Server>>, String> {
        let mut socket = Socket::new_server(addr, CertStrategy::SelfSigned)
            .map_err(|error| format!("failed to bind CEF server on {addr}: {error}"))?;

        let (sender, receiver) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();

        let server = Server {
            event_tx,
            event_rx,
            sender,
            allowed: HashMap::new(),
            clients: HashMap::new(),
            players: HashMap::new(),
            asset_bundle: None,
        };

        let server = Arc::new(Mutex::new(server));
        let server_clone = server.clone();

        std::thread::spawn(move || {
            loop {
                while let Some(event) = socket.recv() {
                    match event {
                        SocketEvent::Message(peer, bytes) => {
                            if let Ok(proto) = deserialize_from_slice::<packets::Packet>(&bytes) {
                                let mut server = server.lock().unwrap();
                                server.handle_client_packet(peer, proto);
                            }
                        }

                        SocketEvent::Connected(peer, addr) => {
                            let mut server = server.lock().unwrap();
                            server.handle_new_connection(peer, addr);
                        }

                        SocketEvent::Disconnect(peer, _) => {
                            let mut server = server.lock().unwrap();
                            server.handle_timeout(peer);
                        }

                        _ => (),
                    }
                }

                for packet in receiver.try_iter() {
                    match packet {
                        Packet::Normal { peer, bytes } => socket.send_message(peer, bytes),
                        Packet::Disconnect(peer) => {
                            trace!("socket::disconnect {:?}", peer);
                            socket.disconnect(peer);
                        }
                    }
                }

                {
                    let mut server = server.lock().unwrap();
                    server.check_bundle_timeouts();
                }

                std::thread::sleep(Duration::from_millis(5));
            }
        });

        Ok(server_clone)
    }

    fn handle_client_packet(&mut self, peer: PeerId, packet: packets::Packet) {
        use messages::packets::PacketId;

        if !self.clients.contains_key(&peer) {
            return;
        }

        match packet.packet_id {
            PacketId::REQUEST_JOIN => {
                let _ = deserialize_from_slice(&packet.bytes)
                    .map(|packet| self.handle_auth(peer, packet));
            }

            PacketId::EMIT_EVENT => {
                let _ = deserialize_from_slice(&packet.bytes)
                    .map(|packet| self.handle_emit_event(peer, packet));
            }

            PacketId::BROWSER_CREATED => {
                let _ = deserialize_from_slice(&packet.bytes)
                    .map(|packet| self.handle_browser_created(peer, packet));
            }

            PacketId::REQUEST_ASSET_BUNDLE => {
                let _ = deserialize_from_slice(&packet.bytes)
                    .map(|packet| self.handle_request_asset_bundle(peer, packet));
            }

            PacketId::ASSET_BUNDLE_READY => {
                let _ = deserialize_from_slice(&packet.bytes)
                    .map(|packet| self.handle_asset_bundle_ready(peer, packet));
            }

            _ => (),
        }
    }

    fn handle_auth(&mut self, peer: PeerId, _packet: packets::RequestJoin) {
        // Decide up front whether the gamemode needs to wait for the
        // client to confirm its UI bundle is ready before we tell it this
        // player is CEF-connected. If this server was never given a
        // bundle (dev setup, or not migrated yet), nothing changes from
        // before this feature existed.
        let bundle_required = self.asset_bundle.is_some();

        let (player_id, response) = {
            let client = self.clients.get_mut(&peer).unwrap();
            client.set_state(State::Connected);

            if bundle_required {
                client.start_bundle_pending();
            }

            (
                client.id(),
                packets::JoinResponse {
                    success: true,
                    current_version: None,
                },
            )
        };

        if !bundle_required {
            let _ = self.event_tx.send(ServerEvent::PlayerConnected(player_id));
        }

        let _ = try_into_packet(response).map(|bytes| {
            let packet = Packet::new(peer, bytes);
            let _ = self.sender.send(packet);
        });

        // Tell the client which encrypted UI bundle it should be running
        // and hand it the key over this same (TLS-secured, see
        // `CertStrategy::SelfSigned` above) channel. The client decides on
        // its own whether its cached copy already matches; either way it
        // replies with ASSET_BUNDLE_READY once installed (see
        // handle_asset_bundle_ready), which is what actually unblocks
        // PlayerConnected above when bundle_required is true.
        if let Some(bundle) = &self.asset_bundle {
            let manifest = packets::AssetManifest {
                version: bundle.version,
                sha256: bundle.sha256.clone().into(),
                size: bundle.size,
                key: bundle.key.to_vec().into(),
            };

            let _ = try_into_packet(manifest).map(|bytes| {
                let packet = Packet::new(peer, bytes);
                let _ = self.sender.send(packet);
            });
        }
    }

    fn handle_asset_bundle_ready(&mut self, peer: PeerId, _packet: packets::AssetBundleReady) {
        let Some(client) = self.clients.get_mut(&peer) else {
            return;
        };

        if !client.bundle_pending() {
            // Either we never asked (no bundle configured) or this is a
            // duplicate — either way, nothing to unblock.
            return;
        }

        client.clear_bundle_pending();
        let player_id = client.id();

        trace!(?peer, player_id, "CEF UI asset bundle ready on client");
        let _ = self.event_tx.send(ServerEvent::PlayerConnected(player_id));
    }

    /// Fail-open safety net: if a client never sends ASSET_BUNDLE_READY
    /// (very slow first-time download, or an old client build that
    /// predates this handshake and silently ignores packets it doesn't
    /// recognize), don't leave the player stuck with no CEF forever — let
    /// the gamemode proceed after a generous grace period instead. Called
    /// from the socket thread's poll loop in `Server::new`.
    fn check_bundle_timeouts(&mut self) {
        let now = Instant::now();
        let mut timed_out = Vec::new();

        for (&peer, client) in self.clients.iter() {
            if let Some(since) = client.bundle_pending_since()
                && now.duration_since(since) >= ASSET_BUNDLE_TIMEOUT
            {
                timed_out.push((peer, client.id()));
            }
        }

        for (peer, player_id) in timed_out {
            if let Some(client) = self.clients.get_mut(&peer) {
                client.clear_bundle_pending();
            }

            tracing::warn!(
                player_id,
                timeout_secs = ASSET_BUNDLE_TIMEOUT.as_secs(),
                "CEF UI asset bundle was not confirmed ready in time; \
                 letting the gamemode proceed anyway (slow connection, or \
                 an outdated client build?)"
            );

            let _ = self.event_tx.send(ServerEvent::PlayerConnected(player_id));
        }
    }

    fn handle_request_asset_bundle(&mut self, peer: PeerId, _packet: packets::RequestAssetBundle) {
        let Some(bundle) = self.asset_bundle.clone() else {
            trace!(?peer, "asset bundle requested but none is loaded on the server");
            return;
        };

        let total_size = bundle.size;

        for offset in (0..bundle.ciphertext.len()).step_by(ASSET_CHUNK_SIZE) {
            let end = (offset + ASSET_CHUNK_SIZE).min(bundle.ciphertext.len());
            let chunk = packets::AssetChunk {
                offset: offset as u32,
                total_size,
                data: bundle.ciphertext[offset..end].to_vec().into(),
            };

            if let Ok(bytes) = try_into_packet(chunk) {
                let packet = Packet::new(peer, bytes);
                let _ = self.sender.send(packet);
            }
        }

        trace!(?peer, size = total_size, "streamed CEF UI asset bundle");
    }

    /// Installs (or replaces) the UI asset bundle served to clients on
    /// join. Called once at startup from `ServerCore::load_asset_bundle`.
    pub fn set_asset_bundle(&mut self, info: Arc<AssetBundleInfo>) {
        self.asset_bundle = Some(info);
    }

    fn handle_emit_event(&mut self, peer: PeerId, packet: packets::EmitEvent) {
        let client = self.clients.get_mut(&peer).unwrap();
        let player_id = client.id();

        if let Some(args) = &packet.args {
            let event = packet.event_name.to_string();
            let arguments = args.to_string();
            let event = ServerEvent::EmitEvent {
                player_id,
                arguments,
                event,
            };

            let _ = self.event_tx.send(event);
        }
    }

    fn handle_browser_created(&mut self, peer: PeerId, packet: packets::BrowserCreated) {
        let client = self.clients.get_mut(&peer).unwrap();
        let player_id = client.id();

        let event = ServerEvent::BrowserCreated {
            player_id,
            browser_id: packet.browser_id,
            code: packet.status_code,
        };

        let _ = self.event_tx.send(event);
    }

    fn handle_timeout(&mut self, addr: PeerId) {
        trace!("handle_timeout {:?}", addr);
        if let Some(client) = self.clients.remove(&addr) {
            self.players.remove(&client.id());
        }

        trace!("{:#?}", self.allowed);
        trace!("{:#?}", self.clients);
    }

    fn handle_new_connection(&mut self, peer: PeerId, addr: SocketAddr) {
        trace!("handle_new_connection {:?} {:?}", peer, addr);

        if !self.clients.contains_key(&peer) && self.allowed.contains_key(&addr.ip()) {
            let player_id = *self.allowed.get(&addr.ip()).unwrap();

            trace!("handle_new_connection: ok {}", player_id);

            if self.peer_by_id(player_id).is_none() {
                trace!("handle_new_connection: ok no peer with this id");

                let client = Client::new(player_id, peer, addr);

                self.clients.insert(peer, client);
                self.players.insert(player_id, peer);

                let request = packets::OpenConnection {};

                let _ = try_into_packet(request).map(|bytes| {
                    let packet = Packet::new(peer, bytes);
                    let _ = self.sender.send(packet);
                });

                return;
            }
        }

        let packet = Packet::disconnect(peer);
        let _ = self.sender.send(packet);
    }

    pub fn allow_connection(&mut self, player_id: i32, addr: IpAddr) {
        if let Some(peer) = self.peer_by_id(player_id) {
            let same_addr = self
                .clients
                .get(&peer)
                .map(|client| client.addr().ip() == addr)
                .unwrap_or(false);

            if !same_addr {
                self.clients.remove(&peer);
                self.players.remove(&player_id);
            }
        }

        self.allowed.insert(addr, player_id);
    }

    pub fn remove_connection(&mut self, player_id: i32, addr: Option<IpAddr>) {
        let peer = self.peer_by_id(player_id);

        if let Some(peer) = peer
            && let Some(client) = self.clients.remove(&peer)
        {
            self.players.remove(&player_id);
            self.allowed.remove(&client.addr().ip());
            let _ = self.sender.send(Packet::Disconnect(client.peer()));
        }

        if let Some(addr) = addr {
            self.allowed.remove(&addr);
        }
    }

    pub fn create_browser(
        &mut self, player_id: i32, browser_id: i32, url: String, hidden: bool, focused: bool,
    ) {
        self.send_packet(
            player_id,
            packets::CreateBrowser {
                browser_id: browser_id as u32,
                url: url.into(),
                hidden,
                focused,
            },
        );
    }

    pub fn destroy_browser(&mut self, player_id: i32, browser_id: i32) {
        self.send_packet(
            player_id,
            packets::DestroyBrowser {
                browser_id: browser_id as u32,
            },
        );
    }

    pub fn hide_browser(&self, player_id: i32, browser_id: i32, hide: bool) {
        self.send_packet(
            player_id,
            packets::HideBrowser {
                browser_id: browser_id as u32,
                hide,
            },
        );
    }

    pub fn focus_browser(&self, player_id: i32, browser_id: i32, focused: bool) {
        self.send_packet(
            player_id,
            packets::FocusBrowser {
                browser_id: browser_id as u32,
                focused,
            },
        );
    }

    pub fn emit_event(&self, player_id: i32, event: &str, arguments: Vec<packets::EventValue>) {
        self.send_packet(
            player_id,
            packets::EmitEvent {
                event_name: event.into(),
                args: None,
                arguments,
            },
        );
    }

    pub fn always_listen_keys(&self, player_id: i32, browser_id: i32, listen: bool) {
        self.send_packet(
            player_id,
            packets::AlwaysListenKeys {
                browser_id: browser_id as u32,
                listen,
            },
        );
    }

    pub fn has_plugin(&self, player_id: i32) -> bool {
        self.peer_by_id(player_id)
            .map(|addr| self.clients.contains_key(&addr))
            .unwrap_or(false)
    }

    pub fn create_external_browser(
        &self, player_id: i32, browser_id: i32, texture: String, url: String, scale: i32,
    ) {
        self.send_packet(
            player_id,
            packets::CreateExternalBrowser {
                browser_id: browser_id as u32,
                url: url.into(),
                texture: texture.into(),
                scale,
            },
        );
    }

    pub fn append_to_object(&self, player_id: i32, browser_id: i32, object_id: i32) {
        self.send_packet(
            player_id,
            packets::AppendToObject {
                browser_id: browser_id as u32,
                object_id,
            },
        );
    }

    pub fn remove_from_object(&self, player_id: i32, browser_id: i32, object_id: i32) {
        self.send_packet(
            player_id,
            packets::RemoveFromObject {
                browser_id: browser_id as u32,
                object_id,
            },
        );
    }

    pub fn toggle_dev_tools(&self, player_id: i32, browser_id: i32, enabled: bool) {
        self.send_packet(
            player_id,
            packets::ToggleDevTools {
                browser_id: browser_id as u32,
                enabled,
            },
        );
    }

    pub fn set_audio_settings(
        &self, player_id: i32, browser_id: u32, max_distance: f32, reference_distance: f32,
    ) {
        self.send_packet(
            player_id,
            packets::SetAudioSettings {
                browser_id,
                max_distance,
                reference_distance,
            },
        );
    }

    pub fn load_url(&self, player_id: i32, browser_id: u32, url: String) {
        self.send_packet(
            player_id,
            packets::LoadUrl {
                browser_id,
                url: url.into(),
            },
        );
    }

    pub fn receiver(&self) -> Receiver<ServerEvent> {
        self.event_rx.clone()
    }

    fn send_packet<'a, T: TryInto<packets::Packet<'a>, Error = quick_protobuf::Error>>(
        &self, player_id: i32, packet: T,
    ) {
        if let Some(addr) = self.peer_by_id(player_id) {
            let Server {
                clients, sender, ..
            } = self;

            if let Some(client) = clients.get(&addr)
                && let Ok(bytes) = try_into_packet(packet)
            {
                let packet = Packet::new(client.peer(), bytes);
                let _ = sender.send(packet);
            }
        }
    }

    fn peer_by_id(&self, player_id: i32) -> Option<PeerId> {
        self.players.get(&player_id).copied()
    }
}
