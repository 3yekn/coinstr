// Copyright (c) 2022-2023 Coinstr
// Distributed under the MIT software license

use std::collections::HashSet;
use std::ops::Add;
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use async_utility::thread;
use coinstr_core::bitcoin::secp256k1::{SecretKey, XOnlyPublicKey};
use coinstr_core::util::Serde;
use coinstr_core::{ApprovedProposal, CompletedProposal, Policy, Proposal, SharedSigner, Signer};
use futures_util::stream::AbortHandle;
use nostr_sdk::nips::nip04;
use nostr_sdk::nips::nip46::{Message as NIP46Message, Request as NIP46Request};
use nostr_sdk::{
    Event, EventBuilder, Filter, Keys, Kind, Metadata, RelayMessage, RelayPoolNotification, Result,
    Tag, TagKind, Timestamp,
};
use tokio::sync::broadcast::Receiver;

use super::{Coinstr, Error, Message};
use crate::constants::{
    APPROVED_PROPOSAL_KIND, COMPLETED_PROPOSAL_KIND, POLICY_KIND, PROPOSAL_KIND, SHARED_KEY_KIND,
    SHARED_SIGNERS_KIND, SIGNERS_KIND,
};
use crate::util::encryption::EncryptionWithKeys;
use crate::{util, Notification};

impl Coinstr {
    fn sync_with_timechain(&self) -> AbortHandle {
        let this = self.clone();
        thread::abortable(async move {
            loop {
                match this.config.electrum_endpoint() {
                    Ok(endpoint) => match this.db.sync_with_timechain(
                        endpoint,
                        this.config.proxy().ok(),
                        &this.sync_channel,
                    ) {
                        Ok(_) => (),
                        Err(e) => log::error!("Impossible to sync wallets: {e}"),
                    },
                    Err(e) => log::error!("Impossible to sync wallets: {e}"),
                }

                thread::sleep(Duration::from_secs(10)).await;
            }
        })
    }

    fn handle_pending_events(&self) -> AbortHandle {
        let this = self.clone();
        thread::abortable(async move {
            loop {
                match this.db.get_pending_events() {
                    Ok(events) => {
                        for event in events.into_iter() {
                            let event_id = event.id;
                            match this.handle_event(event).await {
                                Ok(notification) => {
                                    this.sync_channel.send(notification).ok();
                                }
                                Err(e) => {
                                    log::error!(
                                        "Impossible to handle pending event {event_id}: {e}"
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => log::error!("Impossible to get pending events: {e}"),
                }
                thread::sleep(Duration::from_secs(30)).await;
            }
        })
    }

    fn sync_metadata(&self) -> AbortHandle {
        let this = self.clone();
        thread::abortable(async move {
            loop {
                match this.db.get_unsynced_metadata_pubkeys() {
                    Ok(public_keys) => {
                        if !public_keys.is_empty() {
                            let timeout = Duration::from_secs(10 * public_keys.len() as u64);
                            let filter = Filter::new()
                                .kind(Kind::Metadata)
                                .authors(public_keys.into_iter().map(|p| p.to_string()).collect());
                            this.client.req_events_of(vec![filter], Some(timeout)).await;
                        } else {
                            log::debug!("No public keys metadata to sync")
                        }
                    }
                    Err(e) => log::error!("Impossible to get unsynced metadata public keys: {e}"),
                }
                thread::sleep(Duration::from_secs(60)).await;
            }
        })
    }

    fn rebroadcaster(&self) -> AbortHandle {
        let this = self.clone();
        thread::abortable(async move {
            loop {
                // TODO: check last rebroadcast timestamp from db
                if false {
                    match this.rebroadcast_all_events().await {
                        Ok(_) => log::info!("All events rebroadcasted to relays"),
                        Err(e) => log::error!("Impossible to rebroadcast events to relays: {e}"),
                    }
                }
                thread::sleep(Duration::from_secs(60)).await;
            }
        })
    }

    pub fn sync_notifications(&self) -> Receiver<Option<Message>> {
        self.sync_channel.subscribe()
    }

    pub(crate) fn sync_filters(&self, since: Timestamp) -> Vec<Filter> {
        let base_filter = Filter::new().kinds(vec![
            POLICY_KIND,
            PROPOSAL_KIND,
            APPROVED_PROPOSAL_KIND,
            COMPLETED_PROPOSAL_KIND,
            SHARED_KEY_KIND,
            SIGNERS_KIND,
            SHARED_SIGNERS_KIND,
            Kind::EventDeletion,
        ]);

        let keys = self.client.keys();

        let author_filter = base_filter
            .clone()
            .author(keys.public_key().to_string())
            .since(since);
        let pubkey_filter = base_filter.pubkey(keys.public_key()).since(since);
        let nostr_connect_filter = Filter::new()
            .pubkey(keys.public_key())
            .kind(Kind::NostrConnect)
            .since(since);
        let other_filters = Filter::new()
            .author(keys.public_key().to_string())
            .kinds(vec![Kind::Metadata, Kind::ContactList])
            .since(since);

        vec![
            author_filter,
            pubkey_filter,
            nostr_connect_filter,
            other_filters,
        ]
    }

    pub(crate) fn sync(&self) {
        if self.syncing.load(Ordering::SeqCst) {
            log::warn!("Syncing threads are already running");
        } else {
            let _ = self
                .syncing
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |_| Some(true));
            let this = self.clone();
            thread::spawn(async move {
                // Sync timechain
                let timechain_sync: AbortHandle = this.sync_with_timechain();

                // Pending events handler
                let pending_event_handler = this.handle_pending_events();
                let metadata_sync = this.sync_metadata();

                // Rebroadcaster
                let rebroadcaster = this.rebroadcaster();

                for (relay_url, relay) in this.client.relays().await {
                    let last_sync: Timestamp = match this.db.get_last_relay_sync(&relay_url) {
                        Ok(ts) => ts,
                        Err(e) => {
                            log::error!("Impossible to get last relay sync: {e}");
                            Timestamp::from(0)
                        }
                    };
                    let filters = this.sync_filters(last_sync);
                    if let Err(e) = relay.subscribe(filters, None).await {
                        log::error!("Impossible to subscribe to {relay_url}: {e}");
                    }
                }

                let _ = this
                    .client
                    .handle_notifications(|notification| async {
                        match notification {
                            RelayPoolNotification::Event(_, event) => {
                                let event_id = event.id;
                                if event.is_expired() {
                                    log::warn!("Event {event_id} expired");
                                } else {
                                    match this.handle_event(event).await {
                                        Ok(notification) => {
                                            this.sync_channel.send(notification).ok();
                                        }
                                        Err(e) => {
                                            log::error!("Impossible to handle event {event_id}: {e}");
                                        }
                                    }
                                }
                            }
                            RelayPoolNotification::Message(relay_url, relay_msg) => {
                                if let RelayMessage::EndOfStoredEvents(subscription_id) = relay_msg {
                                    log::debug!("Received new EOSE for {relay_url} with subid {subscription_id}");
                                    if let Ok(relay) = this.client.relay(&relay_url).await {
                                        let subscription = relay.subscription().await;
                                        if subscription.id() == subscription_id {
                                            if let Err(e) = this
                                                .db
                                                .save_last_relay_sync(&relay_url, Timestamp::now())
                                            {
                                                log::error!("Impossible to save last relay sync: {e}");
                                            }
                                        }
                                    }
                                }
                            }
                            RelayPoolNotification::Stop | RelayPoolNotification::Shutdown => {
                                log::debug!("Received stop/shutdown msg");
                                timechain_sync.abort();
                                pending_event_handler.abort();
                                metadata_sync.abort();
                                rebroadcaster.abort();
                                let _ = this.syncing.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |_| Some(false));
                            }
                        }

                        Ok(false)
                    })
                    .await;
                log::debug!("Exited from nostr sync thread");
            });
        }
    }

    async fn handle_event(&self, event: Event) -> Result<Option<Message>> {
        if self.db.event_was_deleted(event.id)? {
            log::warn!("Received an event that was deleted: {}", event.id);
            return Ok(None);
        }

        if event.kind != Kind::NostrConnect {
            if let Err(e) = self.db.save_event(&event) {
                log::error!("Impossible to save event {}: {e}", event.id);
            }
        }

        if event.kind == SHARED_KEY_KIND {
            let policy_id = util::extract_first_event_id(&event).ok_or(Error::PolicyNotFound)?;
            if !self.db.shared_key_exists_for_policy(policy_id)? {
                let keys = self.client.keys();
                let content = nip04::decrypt(&keys.secret_key()?, &event.pubkey, &event.content)?;
                let sk = SecretKey::from_str(&content)?;
                let shared_key = Keys::new(sk);
                self.db.save_shared_key(policy_id, shared_key)?;
            }
        } else if event.kind == POLICY_KIND && !self.db.policy_exists(event.id)? {
            if let Ok(shared_key) = self.db.get_shared_key(event.id) {
                let policy = Policy::decrypt_with_keys(&shared_key, &event.content)?;
                let mut nostr_pubkeys: Vec<XOnlyPublicKey> = Vec::new();
                for tag in event.tags.iter() {
                    if let Tag::PubKey(pubkey, ..) = tag {
                        nostr_pubkeys.push(*pubkey);
                    }
                }
                if nostr_pubkeys.is_empty() {
                    log::error!("Policy {} not contains any nostr pubkey", event.id);
                } else {
                    self.db.save_policy(event.id, policy, nostr_pubkeys)?;
                    let notification = Notification::NewPolicy(event.id);
                    self.db.save_notification(event.id, notification)?;
                    return Ok(Some(Message::Notification(notification)));
                }
            } else {
                self.db.save_pending_event(&event)?;
            }
        } else if event.kind == PROPOSAL_KIND && !self.db.proposal_exists(event.id)? {
            if let Some(policy_id) = util::extract_first_event_id(&event) {
                if let Ok(shared_key) = self.db.get_shared_key(policy_id) {
                    let proposal = Proposal::decrypt_with_keys(&shared_key, &event.content)?;
                    self.db.save_proposal(event.id, policy_id, proposal)?;
                    let notification = Notification::NewProposal(event.id);
                    self.db.save_notification(event.id, notification)?;
                    return Ok(Some(Message::Notification(notification)));
                } else {
                    self.db.save_pending_event(&event)?;
                }
            } else {
                log::error!("Impossible to find policy id in proposal {}", event.id);
            }
        } else if event.kind == APPROVED_PROPOSAL_KIND
            && !self.db.approved_proposal_exists(event.id)?
        {
            if let Some(proposal_id) = util::extract_first_event_id(&event) {
                if let Some(Tag::Event(policy_id, ..)) =
                    util::extract_tags_by_kind(&event, TagKind::E).get(1)
                {
                    if let Ok(shared_key) = self.db.get_shared_key(*policy_id) {
                        let approved_proposal =
                            ApprovedProposal::decrypt_with_keys(&shared_key, &event.content)?;
                        self.db.save_approved_proposal(
                            proposal_id,
                            event.pubkey,
                            event.id,
                            approved_proposal,
                            event.created_at,
                        )?;
                        let notification = Notification::NewApproval {
                            proposal_id,
                            public_key: event.pubkey,
                        };
                        self.db.save_notification(event.id, notification)?;
                        return Ok(Some(Message::Notification(notification)));
                    } else {
                        self.db.save_pending_event(&event)?;
                    }
                } else {
                    log::error!("Impossible to find policy id in proposal {}", event.id);
                }
            } else {
                log::error!(
                    "Impossible to find proposal id in approved proposal {}",
                    event.id
                );
            }
        } else if event.kind == COMPLETED_PROPOSAL_KIND
            && !self.db.completed_proposal_exists(event.id)?
        {
            if let Some(proposal_id) = util::extract_first_event_id(&event) {
                self.db.delete_proposal(proposal_id)?;
                if let Some(Tag::Event(policy_id, ..)) =
                    util::extract_tags_by_kind(&event, TagKind::E).get(1)
                {
                    // Schedule policy for sync if the event was created in the last 600 secs
                    if event.created_at.add(Duration::from_secs(600)) >= Timestamp::now() {
                        self.db.schedule_for_sync(*policy_id)?;
                    }

                    if let Ok(shared_key) = self.db.get_shared_key(*policy_id) {
                        let completed_proposal =
                            CompletedProposal::decrypt_with_keys(&shared_key, &event.content)?;
                        self.db.save_completed_proposal(
                            event.id,
                            *policy_id,
                            completed_proposal,
                        )?;
                        let notification = Notification::NewCompletedProposal(event.id);
                        self.db.save_notification(event.id, notification)?;
                        return Ok(Some(Message::Notification(notification)));
                    } else {
                        self.db.save_pending_event(&event)?;
                    }
                } else {
                    log::error!(
                        "Impossible to find policy id in completed proposal {}",
                        event.id
                    );
                }
            }
        } else if event.kind == SIGNERS_KIND {
            let keys = self.client.keys();
            let signer = Signer::decrypt_with_keys(&keys, event.content)?;
            self.db.save_signer(event.id, signer)?;
        } else if event.kind == SHARED_SIGNERS_KIND {
            let public_key =
                util::extract_first_public_key(&event).ok_or(Error::PublicKeyNotFound)?;
            let keys = self.client.keys();
            if event.pubkey == keys.public_key() {
                let signer_id =
                    util::extract_first_event_id(&event).ok_or(Error::SignerIdNotFound)?;
                self.db
                    .save_my_shared_signer(signer_id, event.id, public_key)?;
            } else {
                let shared_signer =
                    nip04::decrypt(&keys.secret_key()?, &event.pubkey, event.content)?;
                let shared_signer = SharedSigner::from_json(shared_signer)?;
                self.db
                    .save_shared_signer(event.id, event.pubkey, shared_signer)?;
                let notification = Notification::NewSharedSigner {
                    shared_signer_id: event.id,
                    owner_public_key: event.pubkey,
                };
                self.db.save_notification(event.id, notification)?;
                return Ok(Some(Message::Notification(notification)));
            }
        } else if event.kind == Kind::EventDeletion {
            for tag in event.tags.iter() {
                if let Tag::Event(event_id, ..) = tag {
                    if let Ok(Event { pubkey, .. }) = self.db.get_event_by_id(*event_id) {
                        if pubkey == event.pubkey {
                            self.db.delete_generic_event_id(*event_id)?;
                        } else {
                            log::warn!(
                                "{pubkey} tried to delete an event not owned by him: {event_id}"
                            );
                        }
                    }
                }
            }
        } else if event.kind == Kind::ContactList {
            let mut contacts = HashSet::new();
            for tag in event.tags.into_iter() {
                if let Tag::ContactList { pk, .. } = tag {
                    contacts.insert(pk);
                }
            }
            self.db.save_contacts(contacts)?;
        } else if event.kind == Kind::Metadata {
            let metadata = Metadata::from_json(event.content)?;
            self.db.set_metadata(event.pubkey, metadata)?;
        } else if event.kind == Kind::NostrConnect
            && self.db.nostr_connect_session_exists(event.pubkey)?
        {
            let keys = self.client.keys();
            let content = nip04::decrypt(&keys.secret_key()?, &event.pubkey, event.content)?;
            let msg = NIP46Message::from_json(content)?;
            if let Ok(request) = msg.to_request() {
                match request {
                    NIP46Request::Disconnect => {
                        self._disconnect_nostr_connect_session(event.pubkey, None)
                            .await?;
                    }
                    NIP46Request::GetPublicKey => {
                        let uri = self.db.get_nostr_connect_session(event.pubkey)?;
                        let msg = msg
                            .generate_response(&keys)?
                            .ok_or(Error::CantGenerateNostrConnectResponse)?;
                        let nip46_event = EventBuilder::nostr_connect(&keys, uri.public_key, msg)?
                            .to_event(&keys)?;
                        self.client
                            .send_event_to_with_custom_wait(uri.relay_url, nip46_event, None)
                            .await?;
                    }
                    _ => {
                        if self
                            .db
                            .is_nostr_connect_session_pre_authorized(event.pubkey)
                        {
                            let uri = self.db.get_nostr_connect_session(event.pubkey)?;
                            let keys = self.client.keys();
                            let req_message = msg.clone();
                            let msg = msg
                                .generate_response(&keys)?
                                .ok_or(Error::CantGenerateNostrConnectResponse)?;
                            let nip46_event =
                                EventBuilder::nostr_connect(&keys, uri.public_key, msg)?
                                    .to_event(&keys)?;
                            self.client
                                .send_event_to_with_custom_wait(uri.relay_url, nip46_event, None)
                                .await?;
                            self.db.save_nostr_connect_request(
                                event.id,
                                event.pubkey,
                                req_message,
                                event.created_at,
                                true,
                            )?;
                            log::info!(
                                "Auto approved nostr connect request {} for app {}",
                                event.id,
                                event.pubkey
                            )
                        } else {
                            self.db.save_nostr_connect_request(
                                event.id,
                                event.pubkey,
                                msg,
                                event.created_at,
                                false,
                            )?;
                            // TODO: save/send notification
                        }
                    }
                };
            }
        }

        Ok(None)
    }
}
