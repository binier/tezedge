use std::io::{self, Write};

use tezos_messages::p2p::encoding::ack::AckMessage;
use tla_sm::{Proposal, Acceptor};
use crate::{TezedgeState, HandshakeMessageType, Handshake, HandshakeStep, RequestState};
use crate::proposals::PeerWritableProposal;
use crate::chunking::{ChunkWriter, WriteMessageError};

impl<'a, W> Acceptor<PeerWritableProposal<'a, W>> for TezedgeState
    where W: Write,
{
    fn accept(&mut self, proposal: PeerWritableProposal<W>) {
        if let Err(_err) = self.validate_proposal(&proposal) {
            #[cfg(test)]
            assert_ne!(_err, crate::InvalidProposalError::ProposalOutdated);
            return;
        }
        let time = proposal.at;

        if let Some(peer) = self.connected_peers.get_mut(&proposal.peer) {
            loop {
                match peer.write_to(proposal.stream) {
                    Ok(()) => {}
                    Err(WriteMessageError::Empty)
                    | Err(WriteMessageError::Pending) => break,
                    Err(err) => {
                        eprintln!("error while trying to write to peer's stream: {:?}", err);
                        self.blacklist_peer(proposal.at, proposal.peer);
                        break;
                    }
                };
            }
        } else {
            let meta_msg = self.meta_msg();
            let peer = self.pending_peers_mut().and_then(|peers| peers.get_mut(&proposal.peer));
            if let Some(peer) = peer {
                loop {
                    match peer.write_to(proposal.stream) {
                        Ok(msg_type) => {
                            match msg_type {
                                HandshakeMessageType::Connection => {
                                    peer.send_conn_msg_successful(proposal.at);
                                }
                                HandshakeMessageType::Metadata => {
                                    peer.send_meta_msg_successful(proposal.at);
                                }
                                HandshakeMessageType::Ack => {
                                    peer.send_ack_msg_successful(proposal.at);
                                    if peer.handshake.is_finished() {
                                        let peer = self.pending_peers_mut().unwrap()
                                            .remove(&proposal.peer)
                                            .unwrap();
                                        let result = peer.handshake.to_result().unwrap();
                                        self.set_peer_connected(proposal.at, proposal.peer, result);
                                        return self.accept(proposal);
                                    }
                                }
                            }
                        }
                        Err(WriteMessageError::Empty) => {
                            let result = peer.enqueue_send_conn_msg(proposal.at)
                                .and_then(|enqueued| {
                                    if !enqueued {
                                        peer.enqueue_send_meta_msg(proposal.at, meta_msg.clone())
                                    } else {
                                        Ok(enqueued)
                                    }
                                })
                                .and_then(|enqueued| {
                                    if !enqueued {
                                        peer.enqueue_send_ack_msg(proposal.at, AckMessage::Ack)
                                    } else {
                                        Ok(enqueued)
                                    }
                                });
                            match result {
                                Ok(true) => {}
                                Ok(false) => break,
                                Err(err) =>  {
                                    eprintln!("failed to enqueue sending connection message for peer({}): {:?}", proposal.peer, err);
                                    #[cfg(test)]
                                    unreachable!("enqueueing handshake messages should always succeed");
                                    break;
                                }
                            }
                        }
                        Err(WriteMessageError::Pending) => {}
                        Err(err) => {
                            eprintln!("error sending handshake message to peer({}): {:?}", proposal.peer, err);
                            self.blacklist_peer(proposal.at, proposal.peer);
                            break;
                        }
                    };
                }
            }
        }

        self.adjust_p2p_state(time);
        self.periodic_react(time);
    }
}