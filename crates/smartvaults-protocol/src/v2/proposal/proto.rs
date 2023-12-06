// Copyright (c) 2022-2023 Smart Vaults
// Distributed under the MIT software license

use std::str::FromStr;

use nostr::Timestamp;
use smartvaults_core::bitcoin::psbt::PartiallySignedTransaction;
use smartvaults_core::bitcoin::{consensus, Address, Network};
use smartvaults_core::miniscript::Descriptor;

use super::{CompletedProposal, PendingProposal, Period, Proposal, ProposalStatus, Recipient};
use crate::v2::proto::proposal::{
    ProtoCompletedKeyAgentPayment, ProtoCompletedProofOfReserve, ProtoCompletedProposal,
    ProtoCompletedProposalEnum, ProtoCompletedSpending, ProtoPendingKeyAgentPayment,
    ProtoPendingProofOfReserve, ProtoPendingProposal, ProtoPendingProposalEnum,
    ProtoPendingSpending, ProtoPeriod, ProtoProposal, ProtoProposalStatus, ProtoProposalStatusEnum,
    ProtoRecipient,
};
use crate::v2::{Error, NetworkMagic};

impl From<&Recipient> for ProtoRecipient {
    fn from(recipient: &Recipient) -> Self {
        ProtoRecipient {
            address: recipient.address.to_string(),
            amount: recipient.amount,
        }
    }
}

impl From<&Period> for ProtoPeriod {
    fn from(period: &Period) -> Self {
        ProtoPeriod {
            from: period.from.as_u64(),
            to: period.to.as_u64(),
        }
    }
}

impl From<&PendingProposal> for ProtoPendingProposal {
    fn from(value: &PendingProposal) -> Self {
        Self {
            proposal: Some(match value {
                PendingProposal::Spending {
                    descriptor,
                    addresses,
                    description,
                    psbt,
                } => ProtoPendingProposalEnum::Spending(ProtoPendingSpending {
                    descriptor: descriptor.to_string(),
                    addresses: addresses.iter().map(|r| r.into()).collect(),
                    description: description.to_owned(),
                    psbt: psbt.to_string(),
                }),
                PendingProposal::ProofOfReserve {
                    descriptor,
                    message,
                    psbt,
                } => ProtoPendingProposalEnum::ProofOfReserve(ProtoPendingProofOfReserve {
                    descriptor: descriptor.to_string(),
                    message: message.to_owned(),
                    psbt: psbt.to_string(),
                }),
                PendingProposal::KeyAgentPayment {
                    descriptor,
                    signer_descriptor,
                    recipient,
                    period,
                    description,
                    psbt,
                } => ProtoPendingProposalEnum::KeyAgentPayment(ProtoPendingKeyAgentPayment {
                    descriptor: descriptor.to_string(),
                    signer_descriptor: signer_descriptor.to_string(),
                    recipient: Some(recipient.into()),
                    period: Some(period.into()),
                    description: description.to_owned(),
                    psbt: psbt.to_string(),
                }),
            }),
        }
    }
}

impl From<&CompletedProposal> for ProtoCompletedProposal {
    fn from(value: &CompletedProposal) -> Self {
        Self {
            proposal: Some(match value {
                CompletedProposal::Spending { tx } => {
                    ProtoCompletedProposalEnum::Spending(ProtoCompletedSpending {
                        tx: consensus::serialize(tx),
                    })
                }
                CompletedProposal::ProofOfReserve { psbt } => {
                    ProtoCompletedProposalEnum::ProofOfReserve(ProtoCompletedProofOfReserve {
                        psbt: psbt.to_string(),
                    })
                }
                CompletedProposal::KeyAgentPayment { tx } => {
                    ProtoCompletedProposalEnum::KeyAgentPayment(ProtoCompletedKeyAgentPayment {
                        tx: consensus::serialize(tx),
                    })
                }
            }),
        }
    }
}

impl TryFrom<ProtoCompletedProposal> for CompletedProposal {
    type Error = Error;

    fn try_from(value: ProtoCompletedProposal) -> Result<Self, Self::Error> {
        match value
            .proposal
            .ok_or(Error::NotFound(String::from("completed proposal")))?
        {
            ProtoCompletedProposalEnum::Spending(inner) => Ok(Self::Spending {
                tx: consensus::deserialize(&inner.tx)?,
            }),
            ProtoCompletedProposalEnum::ProofOfReserve(inner) => Ok(Self::ProofOfReserve {
                psbt: PartiallySignedTransaction::from_str(&inner.psbt)?,
            }),
            ProtoCompletedProposalEnum::KeyAgentPayment(inner) => Ok(Self::KeyAgentPayment {
                tx: consensus::deserialize(&inner.tx)?,
            }),
        }
    }
}

impl From<&ProposalStatus> for ProtoProposalStatusEnum {
    fn from(value: &ProposalStatus) -> Self {
        match value {
            ProposalStatus::Pending(pending) => Self::Pending(pending.into()),
            ProposalStatus::Completed(completed) => Self::Completed(completed.into()),
        }
    }
}

impl From<&Proposal> for ProtoProposal {
    fn from(proposal: &Proposal) -> Self {
        ProtoProposal {
            status: Some(ProtoProposalStatus {
                proposal: Some((&proposal.status).into()),
            }),
            network: proposal.network.magic().to_bytes().to_vec(),
            timestamp: proposal.timestamp.as_u64(),
        }
    }
}

impl TryFrom<ProtoProposal> for Proposal {
    type Error = Error;

    fn try_from(value: ProtoProposal) -> Result<Self, Self::Error> {
        let network: Network = NetworkMagic::from_slice(&value.network)?.into();
        let status = value
            .status
            .ok_or(Error::NotFound(String::from("proposal status")))?;
        let status = match status
            .proposal
            .ok_or(Error::NotFound(String::from("proposal status enum")))?
        {
            ProtoProposalStatusEnum::Pending(inner) => ProposalStatus::Pending(
                match inner
                    .proposal
                    .ok_or(Error::NotFound(String::from("pending proposal")))?
                {
                    ProtoPendingProposalEnum::Spending(inner) => PendingProposal::Spending {
                        descriptor: Descriptor::from_str(&inner.descriptor)?,
                        addresses: inner
                            .addresses
                            .into_iter()
                            .filter_map(|r| {
                                Some(Recipient {
                                    address: Address::from_str(&r.address)
                                        .ok()?
                                        .require_network(network)
                                        .ok()?,
                                    amount: r.amount,
                                })
                            })
                            .collect(),
                        description: inner.description,
                        psbt: PartiallySignedTransaction::from_str(&inner.psbt)?,
                    },
                    ProtoPendingProposalEnum::ProofOfReserve(inner) => {
                        PendingProposal::ProofOfReserve {
                            descriptor: Descriptor::from_str(&inner.descriptor)?,
                            message: inner.message,
                            psbt: PartiallySignedTransaction::from_str(&inner.psbt)?,
                        }
                    }
                    ProtoPendingProposalEnum::KeyAgentPayment(inner) => {
                        let recipient: ProtoRecipient = inner
                            .recipient
                            .ok_or(Error::NotFound(String::from("recipient")))?;
                        let period: ProtoPeriod = inner
                            .period
                            .ok_or(Error::NotFound(String::from("period")))?;
                        PendingProposal::KeyAgentPayment {
                            descriptor: Descriptor::from_str(&inner.descriptor)?,
                            signer_descriptor: Descriptor::from_str(&inner.signer_descriptor)?,
                            recipient: Recipient {
                                address: Address::from_str(&recipient.address)?
                                    .require_network(network)?,
                                amount: recipient.amount,
                            },
                            period: Period {
                                from: period.from.into(),
                                to: period.to.into(),
                            },
                            description: inner.description,
                            psbt: PartiallySignedTransaction::from_str(&inner.psbt)?,
                        }
                    }
                },
            ),
            ProtoProposalStatusEnum::Completed(inner) => {
                ProposalStatus::Completed(inner.try_into()?)
            }
        };

        Ok(Self {
            status,
            network,
            timestamp: Timestamp::from(value.timestamp),
        })
    }
}
