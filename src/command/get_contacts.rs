use std::{collections::HashMap, time::Duration};

use nostr_sdk::prelude::*;

use crate::{user::User, util::create_client};

#[derive(Debug, Clone, clap::Parser)]
#[command(name = "contacts", about = "Get contacts list from nostr")]
pub struct GetContactsCmd {
	// User name
	#[arg(required = true)]
	user: String,
}

impl GetContactsCmd {
	/// Run the command
	pub fn run(&self, nostr_relay: String) -> Result<()> {
		let relays = vec![nostr_relay];
		let user = User::get(&self.user)?;
		let keys = user.nostr_user.keys;
		let client = create_client(&keys, relays, 0).expect("cannot create client");

		let timeout = Some(Duration::from_secs(60));
		let contacts: HashMap<XOnlyPublicKey, Metadata> =
			client.get_contact_list_metadata(timeout)?;

		for (pubkey, metadata) in contacts.into_iter() {
			println!("Public key: {pubkey}");
			println!("Display name: {}", metadata.display_name.unwrap_or_default());
			println!("Avatar URL: {}", metadata.picture.unwrap_or_default());
			println!("NIP-05: {}", metadata.nip05.unwrap_or_default());
			println!();
		}

		Ok(())
	}
}
