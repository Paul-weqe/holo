//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//

use holo_utils::protocol::Protocol;

use crate::northbound::NorthboundVersion;

// VRRP version-specific code.
pub trait Version
where
    Self: 'static + Send + Default + std::fmt::Debug + NorthboundVersion<Self>,
{
    const PROTOCOL: Protocol;
}

#[derive(Debug, Default)]
pub struct Vrrpv2();

#[derive(Debug, Default)]
pub struct Vrrpv3();

// ==== impl Vrrpv2 ===
impl Version for Vrrpv2 {
    const PROTOCOL: Protocol = Protocol::VRRPV2;
}

impl Version for Vrrpv3 {
    const PROTOCOL: Protocol = Protocol::VRRPV3;
}
