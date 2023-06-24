// Copyright (c) 2022-2023 Coinstr
// Distributed under the MIT software license

use std::collections::BTreeMap;

use bdk::bitcoin::XOnlyPublicKey;
use bdk::Balance;
use coinstr_core::signer::{SharedSigner, Signer};
use coinstr_core::{ApprovedProposal, Policy};
use nostr_sdk::Timestamp;

use crate::types::Notification;

#[derive(Debug, Clone)]
pub struct GetPolicyResult {
    pub policy: Policy,
    pub last_sync: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct GetDetailedPolicyResult {
    pub policy: Policy,
    pub balance: Option<Balance>,
    pub last_sync: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct GetNotificationsResult {
    pub notification: Notification,
    pub timestamp: Timestamp,
    pub seen: bool,
}

#[derive(Debug, Clone)]
pub struct GetApprovedProposalResult {
    pub public_key: XOnlyPublicKey,
    pub approved_proposal: ApprovedProposal,
    pub timestamp: Timestamp,
}

#[derive(Debug, Clone)]
pub struct GetSharedSignerResult {
    pub owner_public_key: XOnlyPublicKey,
    pub shared_signer: SharedSigner,
}

#[derive(Debug, Clone)]
pub struct GetAllSigners {
    pub my: Vec<Signer>,
    pub contacts: BTreeMap<XOnlyPublicKey, Vec<SharedSigner>>,
}
