//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//
// Sponsored by NLnet as part of the Next Generation Internet initiative.
// See: https://nlnet.nl/NGI0
//

use std::borrow::Cow;
use std::sync::atomic::Ordering;
use std::sync::LazyLock as Lazy;

use enum_as_inner::EnumAsInner;
use holo_northbound::state::{
    Callbacks, CallbacksBuilder, ListEntryKind, Provider,
};
use holo_northbound::yang::interfaces;
use holo_utils::option::OptionExt;
use holo_yang::ToYang;

use crate::instance::Instance;
use crate::interface::Interface;
use crate::version::{Version, Vrrpv2, Vrrpv3};

pub static CALLBACKS_VRRPV2: Lazy<Callbacks<Interface<Vrrpv2>>> =
    Lazy::new(load_callbacks_vrrpv2);
pub static CALLBACKS_VRRPV3: Lazy<Callbacks<Interface<Vrrpv3>>> =
    Lazy::new(load_callbacks_vrrpv3);

#[derive(Debug, Default, EnumAsInner)]
pub enum ListEntry<'a> {
    #[default]
    None,
    Instance(u8, &'a Instance),
}

// ===== callbacks =====

fn load_callbacks<V>() -> Callbacks<Interface<V>>
where
    V: Version,
{
    CallbacksBuilder::<Interface<V>>::default()
        .path(interfaces::interface::ipv4::vrrp::vrrp_instance::PATH)
        .get_iterate(|interface, _args| {
            let iter = interface.instances.iter().map(|(vrid, instance)| ListEntry::Instance(*vrid, instance));
            Some(Box::new(iter))
        })
        .get_object(|_interface, args| {
            use interfaces::interface::ipv4::vrrp::vrrp_instance::VrrpInstance;
            let (vrid, instance) = args.list_entry.as_instance().unwrap();
            Box::new(VrrpInstance {
                vrid: *vrid,
                state: Some(instance.state.state.to_yang()),
                // TODO
                is_owner: None,
                last_adv_source: instance.state.last_adv_src.map(std::convert::Into::into).map(Cow::Owned).ignore_in_testing(),
                up_datetime: instance.state.up_time.as_ref().map(Cow::Borrowed).ignore_in_testing(),
                master_down_interval: instance.state.timer.as_master_down_timer().map(|task| task.remaining().as_millis() as u32 / 10).ignore_in_testing(),
                // TODO
                skew_time: None,
                last_event: Some(instance.state.last_event.to_yang()).ignore_in_testing(),
                new_master_reason: Some(instance.state.new_master_reason.to_yang()),
            })
        })
        .path(interfaces::interface::ipv4::vrrp::vrrp_instance::statistics::PATH)
        .get_object(|_interface, args| {
            use interfaces::interface::ipv4::vrrp::vrrp_instance::statistics::Statistics;
            let (_, instance) = args.list_entry.as_instance().unwrap();
            let statistics = &instance.state.statistics;
            Box::new(Statistics {
                discontinuity_datetime: Some(Cow::Borrowed(&statistics.discontinuity_time)).ignore_in_testing(),
                master_transitions: Some(statistics.master_transitions).ignore_in_testing(),
                advertisement_rcvd: Some(statistics.adv_rcvd).ignore_in_testing(),
                advertisement_sent: Some(statistics.adv_sent.load(Ordering::Relaxed)).ignore_in_testing(),
                interval_errors: Some(statistics.interval_errors).ignore_in_testing(),
                priority_zero_pkts_rcvd: Some(statistics.priority_zero_pkts_rcvd).ignore_in_testing(),
                priority_zero_pkts_sent: Some(statistics.priority_zero_pkts_sent).ignore_in_testing(),
                invalid_type_pkts_rcvd: Some(statistics.invalid_type_pkts_rcvd).ignore_in_testing(),
                packet_length_errors: Some(statistics.pkt_length_errors).ignore_in_testing(),
            })
        })
        .build()
}

fn load_callbacks_vrrpv3() -> Callbacks<Interface<Vrrpv3>> {
    let core_cbs = load_callbacks();
    CallbacksBuilder::new(core_cbs).build()
}

fn load_callbacks_vrrpv2() -> Callbacks<Interface<Vrrpv2>> {
    let core_cbs = load_callbacks();
    CallbacksBuilder::new(core_cbs).build()
}

// ===== impl Interface =====

impl<V> Provider for Interface<V>
where
    V: Version,
{
    // TODO
    const STATE_PATH: &'static str = "";

    type ListEntry<'a> = ListEntry<'a>;

    fn callbacks() -> Option<&'static Callbacks<Interface<V>>> {
        V::state_callbacks()
    }
}

// ===== impl ListEntry =====

impl ListEntryKind for ListEntry<'_> {}
