use tla_sm::Acceptor;
use crate::{TezedgeState, Effects};
use crate::proposals::PeerDisconnectedProposal;

impl<E: Effects> Acceptor<PeerDisconnectedProposal> for TezedgeState<E> {
    fn accept(&mut self, proposal: PeerDisconnectedProposal) {
        if let Err(_err) = self.validate_proposal(&proposal) {
            #[cfg(test)]
            assert_ne!(_err, crate::InvalidProposalError::ProposalOutdated);
            return;
        }

        slog::warn!(&self.log, "Peer Disconnected"; "peer_address" => proposal.peer.to_string(), "reason" => "Requested by the Proposer");
        self.disconnect_peer(proposal.at, proposal.peer);

        self.periodic_react(proposal.at);
    }
}