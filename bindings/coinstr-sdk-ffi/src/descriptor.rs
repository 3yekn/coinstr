// Copyright (c) 2022-2023 Coinstr
// Distributed under the MIT software license

#![allow(clippy::should_implement_trait)]
#![allow(clippy::inherent_to_string)]

use std::str::FromStr;

use coinstr_sdk::core::miniscript::DescriptorPublicKey;

use crate::error::Result;

pub struct Descriptor {
    inner: DescriptorPublicKey,
}

impl From<DescriptorPublicKey> for Descriptor {
    fn from(inner: DescriptorPublicKey) -> Self {
        Self { inner }
    }
}

impl Descriptor {
    pub fn from_str(str: String) -> Result<Self> {
        Ok(Self {
            inner: DescriptorPublicKey::from_str(&str)?,
        })
    }

    pub fn to_string(&self) -> String {
        self.inner.to_string()
    }
}
