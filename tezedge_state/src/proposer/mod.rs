use std::collections::VecDeque;
use std::time::{Instant, Duration};
use std::io::{self, Read, Write};
use std::fmt::Debug;
use bytes::Buf;

use crypto::hash::CryptoboxPublicKeyHash;
use tla_sm::{Acceptor, GetRequests};

use tezos_messages::p2p::encoding::peer::PeerMessageResponse;
use tezos_messages::p2p::binary_message::{BinaryMessage, BinaryChunk, BinaryChunkError, CONTENT_LENGTH_FIELD_BYTES};
use tezos_messages::p2p::encoding::prelude::{
    ConnectionMessage, MetadataMessage, AckMessage, NetworkVersion,
};
use crate::{TezedgeStateWrapper, TezedgeState, TezedgeRequest, PeerCrypto, PeerAddress};
use crate::proposals::{
    TickProposal,
    NewPeerConnectProposal,
    PeerReadableProposal,
    PeerDisconnectProposal,
    PeerBlacklistProposal,
    PendingRequestProposal, PendingRequestMsg,
    HandshakeProposal, HandshakeMsg,
};

pub mod mio_manager;

#[derive(Debug)]
pub enum Notification {
    PeerDisconnected { peer: PeerAddress },
    PeerBlacklisted { peer: PeerAddress },
    MessageReceived { peer: PeerAddress, message: PeerMessageResponse },
    HandshakeSuccessful {
        peer_address: PeerAddress,
        peer_public_key_hash: CryptoboxPublicKeyHash,
        metadata: MetadataMessage,
        network_version: NetworkVersion,
    },
}

pub trait GetMessageType {
    fn get_message_type(&self) -> SendMessageType;
}

pub trait AsSendMessage {
    type Error;

    fn as_send_message(&self) -> Result<SendMessage, Self::Error>;
}

pub trait AsEncryptedSendMessage {
    type Error;

    fn as_encrypted_send_message(
        &self,
        crypto: &mut PeerCrypto,
    ) -> Result<SendMessage, Self::Error>;
}

#[derive(Debug, Clone, Copy)]
pub enum SendMessageType {
    Connect,
    Meta,
    Ack,
    Other,
}

#[derive(Debug)]
pub enum SendMessageError {
    IO(io::Error),
    EncodeFailed,
}

impl From<io::Error> for SendMessageError {
    fn from(err: io::Error) -> Self {
        Self::IO(err)
    }
}

#[derive(Debug)]
pub enum SendMessageResult {
    Empty,

    Pending {
        message_type: SendMessageType,
    },

    Ok {
        message_type: SendMessageType,
    },

    Err {
        message_type: SendMessageType,
        error: SendMessageError,
    },
}

impl SendMessageResult {
    pub fn empty() -> Self {
        Self::Empty
    }

    pub fn pending(message_type: SendMessageType) -> Self {
        Self::Pending { message_type }
    }

    pub fn ok(message_type: SendMessageType) -> Self {
        Self::Ok { message_type }
    }

    pub fn err(message_type: SendMessageType, error: SendMessageError) -> Self {
        Self::Err { message_type, error }
    }
}

#[derive(Debug)]
pub struct SendMessage {
    bytes: BinaryChunk,
    message_type: SendMessageType,
}

impl SendMessage {
    fn new(message_type: SendMessageType, bytes: BinaryChunk) -> Self {
        Self { message_type, bytes }
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        self.bytes.raw()
    }

    #[inline]
    fn message_type(&self) -> SendMessageType {
        self.message_type
    }
}

impl GetMessageType for SendMessage {
    fn get_message_type(&self) -> SendMessageType {
        self.message_type()
    }
}


impl GetMessageType for ConnectionMessage {
    fn get_message_type(&self) -> SendMessageType {
        SendMessageType::Connect
    }
}


impl GetMessageType for MetadataMessage {
    fn get_message_type(&self) -> SendMessageType {
        SendMessageType::Meta
    }
}


impl GetMessageType for AckMessage {
    fn get_message_type(&self) -> SendMessageType {
        SendMessageType::Ack
    }
}

impl<M> AsSendMessage for M
    where M: BinaryMessage + GetMessageType
{
    type Error = failure::Error;

    fn as_send_message(&self) -> Result<SendMessage, Self::Error> {
        Ok(SendMessage {
            bytes: BinaryChunk::from_content(&self.as_bytes()?)?,
            message_type: self.get_message_type(),
        })
    }
}

impl AsEncryptedSendMessage for [u8] {
    type Error = failure::Error;

    fn as_encrypted_send_message(
        &self,
        crypto: &mut PeerCrypto,
    ) -> Result<SendMessage, Self::Error>
    {
        let encrypted = crypto.encrypt(&self)?;
        Ok(SendMessage {
            bytes: BinaryChunk::from_content(&encrypted)?,
            message_type: SendMessageType::Other,
        })
    }
}

impl<M> AsEncryptedSendMessage for M
    where M: BinaryMessage + GetMessageType
{
    type Error = failure::Error;

    fn as_encrypted_send_message(
        &self,
        crypto: &mut PeerCrypto,
    ) -> Result<SendMessage, Self::Error>
    {
        let encrypted = crypto.encrypt(
            &self.as_bytes()?,
        )?;
        Ok(SendMessage {
            bytes: BinaryChunk::from_content(&encrypted)?,
            message_type: self.get_message_type(),
        })
    }
}

#[derive(Debug)]
struct WriteBuffer {
    message: SendMessage,
    index: usize,
}

impl WriteBuffer {
    fn new(message: SendMessage) -> Self {
        Self {
            message,
            index: 0,
        }
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        self.message.bytes()
    }

    #[inline]
    fn message_type(&self) -> SendMessageType {
        self.message.message_type()
    }

    fn is_finished(&self) -> bool {
        self.index == self.bytes().len() - 1
    }

    fn next_slice(&self) -> &[u8] {
        &self.bytes()[self.index..]
    }

    fn advance(&mut self, by: usize) {
        self.index = (self.index + by).min(self.bytes().len() - 1);
    }

    fn result_pending(&self) -> SendMessageResult {
        SendMessageResult::pending(self.message_type())
    }

    fn result_ok(&self) -> SendMessageResult {
        SendMessageResult::ok(self.message_type())
    }

    fn result_err(&self, error: SendMessageError) -> SendMessageResult {
        SendMessageResult::err(self.message_type(), error)
    }
}

#[derive(Debug, Clone)]
pub enum Event<NetE> {
    Tick(Instant),
    Network(NetE),
}

impl<NetE> Event<NetE> {
    pub fn as_event_ref<'a>(&'a self) -> EventRef<'a, NetE> {
        match self {
            Self::Tick(e) => EventRef::Tick(*e),
            Self::Network(e) => EventRef::Network(e),
        }
    }
}

pub type EventRef<'a, NetE> = Event<&'a NetE>;

pub trait NetworkEvent {
    fn is_server_event(&self) -> bool;

    fn is_readable(&self) -> bool;
    fn is_writable(&self) -> bool;

    fn is_read_closed(&self) -> bool;
    fn is_write_closed(&self) -> bool;

    fn time(&self) -> Instant {
        Instant::now()
    }
}

pub trait Events {
    fn set_limit(&mut self, limit: usize);
}

pub struct Peer<S> {
    address: PeerAddress,
    pub stream: S,
    write_buf: Option<WriteBuffer>,
    write_queue: VecDeque<SendMessage>,
}

impl<S> Peer<S> {
    pub fn new(address: PeerAddress, stream: S) -> Self {
        Self {
            address,
            stream,
            write_buf: None,
            write_queue: VecDeque::new(),
        }
    }

    pub fn address(&self) -> &PeerAddress {
        &self.address
    }
}

impl<S: Write> Peer<S> {
    pub fn write(&mut self, msg: SendMessage) -> SendMessageResult {
        match self.write_buf.as_mut() {
            Some(_) => {
                self.write_queue.push_back(msg);
                self.try_flush()
            }
            None => {
                self.write_buf.replace(WriteBuffer::new(msg));
                self.try_flush()
            }
        }
    }

    pub fn try_flush(&mut self) -> SendMessageResult {
        let buf = &mut self.write_buf;
        let queue = &mut self.write_queue;
        let stream = &mut self.stream;

        match buf.as_mut() {
            Some(buf) => {
                match self.stream.write(buf.next_slice()) {
                    Ok(size) => {
                        buf.advance(size);
                        if buf.is_finished() {
                            let result = buf.result_ok();
                            self.write_buf.take();
                            let _ = self.stream.flush();
                            result
                        } else {
                            buf.result_pending()
                        }
                    }
                    Err(err) => {
                        match err.kind() {
                            io::ErrorKind::WouldBlock => buf.result_pending(),
                            _ => {
                                let result = buf.result_err(err.into());
                                self.write_buf.take();
                                result
                            }
                        }
                    }
                }
            }
            None => {
                if let Some(msg) = queue.pop_front() {
                    *buf = Some(WriteBuffer::new(msg));
                    self.try_flush()
                } else {
                    SendMessageResult::empty()
                }
            },
        }
    }
}

pub trait Manager {
    type Stream: Read + Write;
    type NetworkEvent: NetworkEvent;
    type Events;

    fn start_listening_to_server_events(&mut self);
    fn stop_listening_to_server_events(&mut self);

    fn accept_connection(&mut self, event: &Self::NetworkEvent) -> Option<&mut Peer<Self::Stream>>;

    fn wait_for_events(&mut self, events_container: &mut Self::Events, timeout: Option<Duration>);

    fn get_peer(&mut self, address: &PeerAddress) -> Option<&mut Peer<Self::Stream>>;
    fn get_peer_or_connect_mut(&mut self, address: &PeerAddress) -> io::Result<&mut Peer<Self::Stream>>;
    fn get_peer_for_event_mut(&mut self, event: &Self::NetworkEvent) -> Option<&mut Peer<Self::Stream>>;

    fn disconnect_peer(&mut self, peer: &PeerAddress);

    fn try_send_msg<M, E>(
        &mut self,
        addr: &PeerAddress,
        msg: M,
    ) -> SendMessageResult
        where M: GetMessageType + AsSendMessage<Error = E>,
              E: Debug,
    {
        let msg = match msg.as_send_message() {
            Ok(msg) => msg,
            Err(err) => {
                eprintln!("failed to encode message: {:?}", err);
                return SendMessageResult::err(
                    msg.get_message_type(),
                    SendMessageError::EncodeFailed,
                );
            }
        };
        match self.get_peer_or_connect_mut(addr) {
            Ok(conn) => conn.write(msg),
            Err(err) => {
                SendMessageResult::err(
                    msg.get_message_type(),
                    SendMessageError::EncodeFailed,
                )
            }
        }
    }

    fn try_send_msg_encrypted<M, E>(
        &mut self,
        addr: &PeerAddress,
        crypto: &mut PeerCrypto,
        msg: M,
    ) -> SendMessageResult
        where M: GetMessageType + AsEncryptedSendMessage<Error = E>,
              E: Debug,
    {
        let msg = match msg.as_encrypted_send_message(crypto) {
            Ok(msg) => msg,
            Err(err) => {
                eprintln!("failed to encode message: {:?}", err);
                return SendMessageResult::err(
                    msg.get_message_type(),
                    SendMessageError::EncodeFailed,
                );
            }
        };
        match self.get_peer_or_connect_mut(addr) {
            Ok(conn) => conn.write(msg),
            Err(err) => {
                SendMessageResult::err(
                    msg.get_message_type(),
                    SendMessageError::EncodeFailed,
                )
            }
        }
    }

}

pub struct TezedgeProposerConfig {
    pub wait_for_events_timeout: Option<Duration>,
    pub events_limit: usize,
}

/// Returns true if it is maybe possible to do further write.
fn handle_send_message_result(
    at: Instant,
    tezedge_state: &mut TezedgeStateWrapper,
    address: PeerAddress,
    result: SendMessageResult,
) -> bool {
    use SendMessageResult::*;
    match result {
        Empty => false,
        Pending { .. } => false,
        Ok { message_type } => {
            let msg = match message_type {
                SendMessageType::Connect => HandshakeMsg::SendConnectSuccess,
                SendMessageType::Meta => HandshakeMsg::SendMetaSuccess,
                SendMessageType::Ack => HandshakeMsg::SendAckSuccess,
                SendMessageType::Other => { return true; }
            };

            tezedge_state.accept(HandshakeProposal {
                at,
                peer: address,
                message: msg,
            });
            true
        }
        Err { message_type, error } => {
            let msg = match message_type {
                SendMessageType::Connect => HandshakeMsg::SendConnectError,
                SendMessageType::Meta => HandshakeMsg::SendMetaError,
                SendMessageType::Ack => HandshakeMsg::SendAckError,
                // TODO temporary panic
                SendMessageType::Other => panic!(),
            };

            tezedge_state.accept(HandshakeProposal {
                at,
                peer: address,
                message: msg,
            });
            true
        }
    }
}

pub struct TezedgeProposer<Es, M> {
    config: TezedgeProposerConfig,
    notifications: Vec<Notification>,
    pub state: TezedgeStateWrapper,
    pub events: Es,
    pub manager: M,
}

impl<Es, M> TezedgeProposer<Es, M>
    where Es: Events,
{
    pub fn new(
        config: TezedgeProposerConfig,
        state: TezedgeState,
        mut events: Es,
        manager: M,
    ) -> Self
    {
        events.set_limit(config.events_limit);
        Self {
            config,
            notifications: vec![],
            state: state.into(),
            events,
            manager,
        }
    }
}

impl<S, NetE, Es, M> TezedgeProposer<Es, M>
    where S: Read + Write,
          NetE: NetworkEvent + Debug,
          M: Manager<Stream = S, NetworkEvent = NetE, Events = Es>,
{
    fn handle_event(
        event: Event<NetE>,
        notifications: &mut Vec<Notification>,
        state: &mut TezedgeStateWrapper,
        manager: &mut M,
    ) {
        match event {
            Event::Tick(at) => {
                state.accept(TickProposal { at });
            }
            Event::Network(event) => {
                Self::handle_network_event(&event, notifications, state, manager);
            }
        }
    }

    fn handle_event_ref<'a>(
        event: EventRef<'a, NetE>,
        notifications: &mut Vec<Notification>,
        state: &mut TezedgeStateWrapper,
        manager: &mut M,
    ) {
        match event {
            Event::Tick(at) => {
                state.accept(TickProposal { at });
            }
            Event::Network(event) => {
                Self::handle_network_event(event, notifications, state, manager);
            }
        }
    }

    fn handle_network_event(
        event: &NetE,
        notifications: &mut Vec<Notification>,
        state: &mut TezedgeStateWrapper,
        manager: &mut M,
    ) {
        if event.is_server_event() {
            // we received event for the server (client opened tcp stream to us).
            loop {
                // as an optimization, execute requests only after 100
                // accepted new connections. We need to execute those
                // requests as they might include command to stop
                // listening for new connections or disconnect new peer,
                // if for example they are blacklisted.
                for _ in 0..100 {
                    match manager.accept_connection(&event) {
                        Some(peer) => {
                            state.accept(NewPeerConnectProposal {
                                at: event.time(),
                                peer: peer.address().clone(),
                            });
                            Self::handle_readiness_event(event, state, peer);
                        }
                        None => return,
                    }
                }
                Self::execute_requests(notifications, state, manager);
            }
        } else {
            match manager.get_peer_for_event_mut(&event) {
                Some(peer) => Self::handle_readiness_event(event, state, peer),
                None => {
                    // TODO: write error log.
                    return;
                }
            }
        };
    }

    fn handle_readiness_event(
        event: &NetE,
        state: &mut TezedgeStateWrapper,
        peer: &mut Peer<S>,
    ) {
        if event.is_read_closed() || event.is_write_closed() {
            state.accept(PeerDisconnectProposal {
                at: event.time(),
                peer: peer.address().clone(),
            });
            return;
        }

        if event.is_readable() {
            state.accept(PeerReadableProposal {
                at: event.time(),
                peer: peer.address().clone(),
                stream: &mut peer.stream,
            });
        }

        if event.is_writable() {
            // flush while it is possble that further progress can be made.
            while handle_send_message_result(
                event.time(),
                state,
                peer.address().clone(),
                peer.try_flush(),
            ) {}
        }
    }

    fn execute_requests(
        notifications: &mut Vec<Notification>,
        state: &mut TezedgeStateWrapper,
        manager: &mut M,
    ) {
        for req in state.get_requests() {
            match req {
                TezedgeRequest::StartListeningForNewPeers { req_id } => {
                    manager.start_listening_to_server_events();
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::StartListeningForNewPeersSuccess,
                    });
                }
                TezedgeRequest::StopListeningForNewPeers { req_id } => {
                    manager.stop_listening_to_server_events();
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::StopListeningForNewPeersSuccess,
                    });
                }
                TezedgeRequest::SendPeerConnect { peer, message } => {
                    let result = manager.try_send_msg(&peer, message);
                    state.accept(HandshakeProposal {
                        at: state.newest_time_seen(),
                        peer: peer.clone(),
                        message: HandshakeMsg::SendConnectPending,
                    });
                    handle_send_message_result(state.newest_time_seen(), state, peer, result);
                }
                TezedgeRequest::SendPeerMeta { peer, message } => {
                    if let Some(crypto) = state.get_peer_crypto(&peer) {
                        let result = manager.try_send_msg_encrypted(&peer, crypto, message);
                        state.accept(HandshakeProposal {
                            at: state.newest_time_seen(),
                            peer: peer.clone(),
                            message: HandshakeMsg::SendMetaPending,
                        });
                        handle_send_message_result(state.newest_time_seen(), state, peer, result);
                    }
                }
                TezedgeRequest::SendPeerAck { peer, message } => {
                    if let Some(crypto) = state.get_peer_crypto(&peer) {
                        let result = manager.try_send_msg_encrypted(&peer,crypto, message);
                        state.accept(HandshakeProposal {
                            at: state.newest_time_seen(),
                            peer: peer.clone(),
                            message: HandshakeMsg::SendAckPending,
                        });
                        handle_send_message_result(state.newest_time_seen(), state, peer, result);
                    }
                }
                TezedgeRequest::DisconnectPeer { req_id, peer } => {
                    manager.disconnect_peer(&peer);
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::DisconnectPeerSuccess,
                    });
                    notifications.push(Notification::PeerDisconnected { peer });
                }
                TezedgeRequest::BlacklistPeer { req_id, peer } => {
                    manager.disconnect_peer(&peer);
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::BlacklistPeerSuccess,
                    });
                    notifications.push(Notification::PeerBlacklisted { peer });
                }
                TezedgeRequest::PeerMessageReceived { req_id, peer, message } => {
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::PeerMessageReceivedNotified,
                    });
                    notifications.push(Notification::MessageReceived { peer, message });
                }
                TezedgeRequest::NotifyHandshakeSuccessful {
                    req_id, peer_address, peer_public_key_hash, metadata, network_version
                } => {
                    notifications.push(Notification::HandshakeSuccessful {
                        peer_address,
                        peer_public_key_hash,
                        metadata,
                        network_version,
                    });
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::HandshakeSuccessfulNotified,
                    });
                }
            }
        }
    }

    fn wait_for_events(&mut self) {
        let wait_for_events_timeout = self.config.wait_for_events_timeout;
        self.manager.wait_for_events(&mut self.events, wait_for_events_timeout)
    }

    pub fn make_progress(&mut self)
        where for<'a> &'a Es: IntoIterator<Item = EventRef<'a, NetE>>,
    {
        self.wait_for_events();

        let events_limit = self.config.events_limit;

        for event in self.events.into_iter().take(events_limit) {
            Self::handle_event_ref(
                event,
                &mut self.notifications,
                &mut self.state,
                &mut self.manager,
            );
        }

        Self::execute_requests(&mut self.notifications, &mut self.state, &mut self.manager);
    }

    pub fn make_progress_owned(&mut self)
        where for<'a> &'a Es: IntoIterator<Item = Event<NetE>>,
    {
        let time = Instant::now();
        self.wait_for_events();
        eprintln!("waited for events for: {}ms", time.elapsed().as_millis());

        let events_limit = self.config.events_limit;

        let time = Instant::now();
        let mut count = 0;
        for event in self.events.into_iter().take(events_limit) {
            count += 1;
            Self::handle_event(
                event,
                &mut self.notifications,
                &mut self.state,
                &mut self.manager,
            );
        }
        eprintln!("handled {} events in: {}ms", count, time.elapsed().as_millis());

        let time = Instant::now();
        Self::execute_requests(&mut self.notifications, &mut self.state, &mut self.manager);
        eprintln!("executed requests in: {}ms", time.elapsed().as_millis());
    }

    pub fn disconnect_peer(&mut self, at: Instant, peer: PeerAddress) {
        self.state.accept(PeerDisconnectProposal { at, peer })
    }

    pub fn blacklist_peer(&mut self, at: Instant, peer: PeerAddress) {
        self.state.accept(PeerBlacklistProposal { at, peer })
    }

    // TODO: Everything bellow this line is temporary until everything
    // is handled in TezedgeState.
    // ---------------------------------------------------------------

    pub fn send_message_to_peer_or_queue(&mut self, addr: PeerAddress, message: &[u8]) {
        if let Some(crypto) = self.state.get_peer_crypto(&addr) {
            if let Ok(msg) = message.as_encrypted_send_message(crypto) {
                if let Some(peer) = self.manager.get_peer(&addr) {
                    peer.write(msg);
                }
            }
        }
    }

    pub fn take_notifications(&mut self) -> Vec<Notification> {
        std::mem::take(&mut self.notifications)
    }
}
