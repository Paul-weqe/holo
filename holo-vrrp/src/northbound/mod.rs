//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//
// Sponsored by NLnet as part of the Next Generation Internet initiative.
// See: https://nlnet.nl/NGI0
//

pub mod configuration;
pub mod notification;
pub mod state;
pub mod yang;

use holo_northbound as northbound;
use holo_northbound::ProviderBase;
use tracing::{debug_span, Span};

use crate::interface::Interface;
use crate::version::{Version, Vrrpv2, Vrrpv3};

// VRRP version-specific code
pub trait NorthboundVersion<V: Version> {
    const STATE_PATH: &'static str = "";

    fn debug_span(interface: &str) -> Span;
    fn validation_callbacks(
    ) -> Option<&'static northbound::configuration::ValidationCallbacks>;
    fn configuration_callbacks(
    ) -> Option<&'static northbound::configuration::Callbacks<Interface<V>>>;
    fn state_callbacks(
    ) -> Option<&'static northbound::state::Callbacks<Interface<V>>>;
}

// ==== impl NorthboundVersion ====

impl NorthboundVersion<Self> for Vrrpv2 {
    fn debug_span(interface: &str) -> Span {
        debug_span!("vrrp-v2", %interface)
    }

    fn validation_callbacks(
    ) -> Option<&'static northbound::configuration::ValidationCallbacks> {
        Some(&configuration::VALIDATION_CALLBACKS_VRRPV2)
    }

    fn configuration_callbacks(
    ) -> Option<&'static northbound::configuration::Callbacks<Interface<Self>>>
    {
        Some(&configuration::CALLBACKS_VRRPV2)
    }

    fn state_callbacks(
    ) -> Option<&'static northbound::state::Callbacks<Interface<Self>>> {
        Some(&state::CALLBACKS_VRRPV2)
    }
}

impl NorthboundVersion<Self> for Vrrpv3 {
    fn debug_span(interface: &str) -> Span {
        debug_span!("vrrp-v3", %interface)
    }

    fn validation_callbacks(
    ) -> Option<&'static northbound::configuration::ValidationCallbacks> {
        Some(&configuration::VALIDATION_CALLBACKS_VRRPV3)
    }

    fn configuration_callbacks(
    ) -> Option<&'static northbound::configuration::Callbacks<Interface<Self>>>
    {
        Some(&configuration::CALLBACKS_VRRPV3)
    }

    fn state_callbacks(
    ) -> Option<&'static northbound::state::Callbacks<Interface<Self>>> {
        Some(&state::CALLBACKS_VRRPV3)
    }
}

// ===== impl Interface =====

impl<V> ProviderBase for Interface<V>
where
    V: Version,
{
    fn yang_modules() -> &'static [&'static str] {
        &["ietf-vrrp", "holo-vrrp"]
    }

    fn top_level_node(&self) -> String {
        "/ietf-interfaces:interfaces".to_owned()
    }

    fn debug_span(interface: &str) -> Span {
        V::debug_span(interface)
    }
}

// No RPC/Actions to implement.
impl<V> holo_northbound::rpc::Provider for Interface<V> where V: Version {}
