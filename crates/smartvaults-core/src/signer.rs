// Copyright (c) 2022-2024 Smart Vaults
// Distributed under the MIT software license

use core::fmt;
use std::collections::BTreeMap;

use keechain_core::bips::bip32::{self, Bip32, ChildNumber, DerivationPath, Fingerprint};
use keechain_core::bips::bip48::ScriptType;
use keechain_core::bitcoin::Network;
use keechain_core::descriptors::{self, ToDescriptor};
use keechain_core::miniscript::DescriptorPublicKey;
use keechain_core::{ColdcardGenericJson, Purpose, Seed};
use thiserror::Error;

use crate::constants::SMARTVAULTS_ACCOUNT_INDEX;
#[cfg(feature = "hwi")]
use crate::hwi::BoxedHWI;
use crate::SECP256K1;

const PURPOSES: [Purpose; 2] = [
    Purpose::BIP48 {
        script: ScriptType::P2WSH,
    },
    Purpose::BIP48 {
        script: ScriptType::P2TR,
    },
];

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    BIP32(#[from] bip32::Error),
    #[error(transparent)]
    Descriptor(#[from] descriptors::Error),
    #[error(transparent)]
    Coldcard(#[from] keechain_core::export::coldcard::Error),
    /// HWI error
    #[cfg(feature = "hwi")]
    #[error(transparent)]
    HWI(#[from] async_hwi::Error),
    #[error("derivation path not found")]
    DerivationPathNotFound,
    #[error("fingerprint not match")]
    FingerprintNotMatch,
    #[error("network not found")]
    NetworkNotFound,
    #[error("network not match")]
    NetworkNotMatch,
    #[error("purpose not found")]
    PurposeNotFound,
    #[error("purpose not match")]
    PurposeNotMatch,
}

/// Signer Type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SignerType {
    /// Seed
    Seed,
    /// Signing Device (aka Hardware Wallet) that can be used
    /// with USB, Bluetooth or other that provides a direct connection with the wallet.
    Hardware,
    /// Signing Device that can be used without ever being connected
    /// to online devices, via microSD or camera.
    AirGap,
    /// Unknown signer type
    Unknown,
}

impl fmt::Display for SignerType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignerType::Seed => write!(f, "Seed"),
            SignerType::Hardware => write!(f, "Hardware"),
            SignerType::AirGap => write!(f, "AirGap"),
            SignerType::Unknown => write!(f, "Unknown"),
        }
    }
}

// TODO: custom impl PartialEq, Eq, PartialOrd, Ord, Hash? Checking only fingerprint and network?
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CoreSigner {
    fingerprint: Fingerprint,
    descriptors: BTreeMap<Purpose, DescriptorPublicKey>,
    r#type: SignerType,
    network: Network,
}

impl CoreSigner {
    pub fn new(
        fingerprint: Fingerprint,
        descriptors: BTreeMap<Purpose, DescriptorPublicKey>,
        r#type: SignerType,
        network: Network,
    ) -> Result<Self, Error> {
        // Compose signer
        let mut signer = Self {
            fingerprint,
            descriptors: BTreeMap::new(),
            r#type,
            network,
        };

        // Add descriptors
        signer.add_descriptors(descriptors)?;

        Ok(signer)
    }

    /// Compose [CoreSigner] from [Seed]
    pub fn from_seed(seed: &Seed, account: Option<u32>, network: Network) -> Result<Self, Error> {
        let mut descriptors: BTreeMap<Purpose, DescriptorPublicKey> = BTreeMap::new();

        let mut purposes: Vec<Purpose> = PURPOSES.to_vec();

        // Check if the account index match the `SMARTVAULTS_ACCOUNT_INDEX` const,
        // to include the BIP86 descriptor to avoid issues with old vaults
        if Some(SMARTVAULTS_ACCOUNT_INDEX) == account {
            purposes.push(Purpose::BIP86);
        }

        // Derive descriptors
        for purpose in purposes.into_iter() {
            let descriptor = seed.to_descriptor(purpose, account, false, network, &SECP256K1)?;
            descriptors.insert(purpose, descriptor);
        }

        Self::new(
            seed.fingerprint(network, &SECP256K1)?,
            descriptors,
            SignerType::Seed,
            network,
        )
    }

    /// Compose [CoreSigner] from custom airgap device
    pub fn airgap(
        fingerprint: Fingerprint,
        descriptors: BTreeMap<Purpose, DescriptorPublicKey>,
        network: Network,
    ) -> Result<Self, Error> {
        Self::new(fingerprint, descriptors, SignerType::AirGap, network)
    }

    /// Compose [CoreSigner] from Coldcard generic JSON (`coldcard-export.json`)
    pub fn from_coldcard(coldcard: ColdcardGenericJson, network: Network) -> Result<Self, Error> {
        let mut descriptors: BTreeMap<Purpose, DescriptorPublicKey> = BTreeMap::new();

        // Derive descriptors
        for purpose in PURPOSES.into_iter() {
            let descriptor = coldcard.descriptor(purpose)?;
            descriptors.insert(purpose, descriptor);
        }

        Self::airgap(coldcard.fingerprint(), descriptors, network)
    }

    #[cfg(feature = "hwi")]
    pub async fn from_hwi(device: BoxedHWI, network: Network) -> Result<Self, Error> {
        let root_fingerprint: Fingerprint = device.get_master_fingerprint().await?;

        let mut descriptors: BTreeMap<Purpose, DescriptorPublicKey> = BTreeMap::new();
        for purpose in PURPOSES.into_iter() {
            let path: DerivationPath = purpose.to_account_extended_path(network, None)?;
            let pubkey = device.get_extended_pubkey(&path).await?;
            let (_, descriptor): (_, DescriptorPublicKey) =
                descriptors::descriptor(root_fingerprint, pubkey, &path, false)?;
            descriptors.insert(purpose, descriptor);
        }

        Self::new(root_fingerprint, descriptors, SignerType::Hardware, network)
    }

    /// Compose [CoreSigner] with unknown type
    pub fn unknown(
        fingerprint: Fingerprint,
        descriptors: BTreeMap<Purpose, DescriptorPublicKey>,
        network: Network,
    ) -> Result<Self, Error> {
        Self::new(fingerprint, descriptors, SignerType::Unknown, network)
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Get [CoreSigner] type
    pub fn r#type(&self) -> SignerType {
        self.r#type
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn descriptors(&self) -> &BTreeMap<Purpose, DescriptorPublicKey> {
        &self.descriptors
    }

    pub fn descriptor(&self, purpose: Purpose) -> Option<DescriptorPublicKey> {
        self.descriptors.get(&purpose).cloned()
    }

    /// Add descriptor
    pub fn add_descriptor(
        &mut self,
        purpose: Purpose,
        descriptor: DescriptorPublicKey,
    ) -> Result<(), Error> {
        // Check if fingerprint match
        if self.fingerprint != descriptor.master_fingerprint() {
            return Err(Error::FingerprintNotMatch);
        }

        // Get derivation path
        let path: DerivationPath = descriptor
            .full_derivation_path()
            .ok_or(Error::DerivationPathNotFound)?;
        let mut path_iter = path.into_iter();

        // Check purpose
        let purp = path_iter.next().ok_or(Error::PurposeNotFound)?;
        match purp {
            ChildNumber::Hardened { index } => {
                if *index != purpose.as_u32() {
                    return Err(Error::PurposeNotMatch);
                }
            }
            _ => return Err(Error::PurposeNotMatch),
        };

        // Check network
        let coin: &ChildNumber = path_iter.next().ok_or(Error::NetworkNotFound)?;
        let res: bool = match coin {
            ChildNumber::Hardened { index } => match self.network {
                Network::Bitcoin => *index == 0, // Mainnet
                _ => *index == 1,                // Testnet, Signer or Regtest
            },
            _ => false,
        };

        if !res {
            return Err(Error::NetworkNotMatch);
        }

        // Insert descriptor
        self.descriptors.insert(purpose, descriptor);

        Ok(())
    }

    pub fn add_descriptors<I>(&mut self, descriptors: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = (Purpose, DescriptorPublicKey)>,
    {
        for (purpose, descriptor) in descriptors.into_iter() {
            self.add_descriptor(purpose, descriptor)?;
        }
        Ok(())
    }

    /// Check if signer contains [`DescriptorPublicKey`]
    pub fn contains_descriptor(&self, descriptor: &DescriptorPublicKey) -> bool {
        self.descriptors.values().any(|d| d == descriptor)
    }
}
