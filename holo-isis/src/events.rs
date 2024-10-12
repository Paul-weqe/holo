//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//
// Sponsored by NLnet as part of the Next Generation Internet initiative.
// See: https://nlnet.nl/NGI0
//

use std::cmp::Ordering;
use std::collections::HashMap;

use bytes::Bytes;
use chrono::Utc;

use crate::adjacency::{Adjacency, AdjacencyEvent, AdjacencyState};
use crate::collections::{
    AdjacencyKey, InterfaceIndex, InterfaceKey, LspEntryKey,
};
use crate::debug::{Debug, LspPurgeReason};
use crate::error::{AdjacencyRejectError, Error};
use crate::instance::{InstanceArenas, InstanceUpView};
use crate::interface::InterfaceType;
use crate::lsdb::{self, lsp_compare, LspEntryFlags};
use crate::northbound::notification;
use crate::packet::consts::PduType;
use crate::packet::error::{DecodeError, DecodeResult};
use crate::packet::pdu::{Hello, HelloVariant, Lsp, Pdu, Snp, SnpTlvs};
use crate::packet::tlv::LspEntry;
use crate::packet::{LanId, LevelNumber, LevelType, LspId};
use crate::tasks;
use crate::tasks::messages::input::DisElectionMsg;

// ===== Network PDU receipt =====

pub(crate) fn process_pdu(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_key: InterfaceKey,
    src: [u8; 6],
    bytes: Bytes,
    pdu: DecodeResult<Pdu>,
) -> Result<(), Error> {
    // Lookup interface.
    let (iface_idx, iface) = arenas.interfaces.get_mut_by_key(&iface_key)?;

    // Ignore PDUs received on inactive or passive interfaces.
    if !iface.state.active || iface.is_passive() {
        return Ok(());
    }

    // Check if the PDU was decoded successfully.
    let pdu = match pdu {
        Ok(pdu) => pdu,
        Err(error) => {
            match error {
                DecodeError::InvalidVersion(version) => {
                    iface.state.event_counters.version_skew += 1;
                    iface.state.discontinuity_time = Utc::now();
                    notification::version_skew(
                        instance, iface, version, &bytes,
                    );
                }
                DecodeError::InvalidIdLength(pdu_id_len) => {
                    iface.state.event_counters.id_len_mismatch += 1;
                    iface.state.discontinuity_time = Utc::now();
                    notification::id_len_mismatch(
                        instance, iface, pdu_id_len, &bytes,
                    );
                }
                DecodeError::UnknownPduType(_) => {
                    iface.state.packet_counters.l1.unknown_in += 1;
                    iface.state.packet_counters.l2.unknown_in += 1;
                    iface.state.discontinuity_time = Utc::now();
                }
                _ => (),
            }
            return Err(Error::PduDecodeError(iface_idx, src, error));
        }
    };

    // Update packet counters.
    match pdu.pdu_type() {
        PduType::HelloP2P => {
            iface.state.packet_counters.l1.iih_in += 1;
            iface.state.packet_counters.l2.iih_in += 1;
        }
        PduType::HelloLanL1 => {
            iface.state.packet_counters.l1.iih_in += 1;
        }
        PduType::HelloLanL2 => {
            iface.state.packet_counters.l2.iih_in += 1;
        }
        PduType::LspL1 => {
            iface.state.packet_counters.l1.lsp_in += 1;
        }
        PduType::LspL2 => {
            iface.state.packet_counters.l2.lsp_in += 1;
        }
        PduType::CsnpL1 => {
            iface.state.packet_counters.l1.csnp_in += 1;
        }
        PduType::CsnpL2 => {
            iface.state.packet_counters.l2.csnp_in += 1;
        }
        PduType::PsnpL1 => {
            iface.state.packet_counters.l1.psnp_in += 1;
        }
        PduType::PsnpL2 => {
            iface.state.packet_counters.l2.psnp_in += 1;
        }
    }
    iface.state.discontinuity_time = Utc::now();

    // Log received PDU.
    Debug::PduRx(iface, &src, &pdu).log();

    match pdu {
        Pdu::Hello(hello) => {
            process_pdu_hello(instance, arenas, iface_idx, src, bytes, hello)
        }
        Pdu::Lsp(lsp) => {
            process_pdu_lsp(instance, arenas, iface_idx, src, bytes, lsp)
        }
        Pdu::Snp(snp) => {
            process_pdu_snp(instance, arenas, iface_idx, src, bytes, snp)
        }
    }
}

pub(crate) fn process_pdu_hello(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_idx: InterfaceIndex,
    src: [u8; 6],
    bytes: Bytes,
    hello: Hello,
) -> Result<(), Error> {
    if let Err(error) = match hello.variant {
        // LAN Hello.
        HelloVariant::Lan { priority, lan_id } => process_pdu_hello_lan(
            instance, arenas, iface_idx, src, hello, priority, lan_id,
        ),
        // Point-to-Point Hello.
        HelloVariant::P2P { .. } => {
            process_pdu_hello_p2p(instance, arenas, iface_idx, src, hello)
        }
    } {
        // Error handling.
        let iface = &mut arenas.interfaces[iface_idx];
        match error {
            AdjacencyRejectError::MaxAreaAddrsMismatch(pdu_max_area_addrs) => {
                iface.state.event_counters.max_area_addr_mismatch += 1;
                iface.state.discontinuity_time = Utc::now();
                notification::max_area_addresses_mismatch(
                    instance,
                    iface,
                    pdu_max_area_addrs,
                    &bytes,
                );
            }
            AdjacencyRejectError::AreaMismatch => {
                iface.state.event_counters.area_mismatch += 1;
                iface.state.discontinuity_time = Utc::now();
                notification::area_mismatch(instance, iface, &bytes);
            }
            _ => {
                iface.state.event_counters.adjacency_rejects += 1;
                iface.state.discontinuity_time = Utc::now();
                notification::rejected_adjacency(
                    instance, iface, &bytes, &error,
                );
            }
        }
        return Err(Error::AdjacencyReject(iface_idx, src, error));
    }

    Ok(())
}

pub(crate) fn process_pdu_hello_lan(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_idx: InterfaceIndex,
    src: [u8; 6],
    hello: Hello,
    priority: u8,
    lan_id: LanId,
) -> Result<(), AdjacencyRejectError> {
    let iface = &mut arenas.interfaces[iface_idx];
    let mut new_adj = false;

    // Validate PDU type and determine level usage.
    let level = match (iface.config.interface_type, hello.hdr.pdu_type) {
        (InterfaceType::Broadcast, PduType::HelloLanL1) => LevelNumber::L1,
        (InterfaceType::Broadcast, PduType::HelloLanL2) => LevelNumber::L2,
        _ => return Err(AdjacencyRejectError::InvalidHelloType),
    };
    if !iface.config.level_type.resolved.intersects(level) {
        return Err(AdjacencyRejectError::InvalidHelloType);
    }

    // Validate the "Circuit Type" field.
    if !iface
        .config
        .level_type
        .resolved
        .intersects(hello.circuit_type)
    {
        return Err(AdjacencyRejectError::CircuitTypeMismatch);
    }

    if hello.hdr.pdu_type == PduType::HelloLanL1 {
        // Validate the "Maximum Area Addresses" field.
        if hello.hdr.max_area_addrs != 0 && hello.hdr.max_area_addrs != 3 {
            return Err(AdjacencyRejectError::MaxAreaAddrsMismatch(
                hello.hdr.max_area_addrs,
            ));
        }

        // Check for area mismatch.
        if !hello
            .tlvs
            .area_addrs()
            .any(|addr| instance.config.area_addrs.contains(addr))
        {
            return Err(AdjacencyRejectError::AreaMismatch);
        }
    }

    // Check for duplicate System-ID.
    if hello.source == instance.config.system_id.unwrap() {
        return Err(AdjacencyRejectError::DuplicateSystemId);
    }

    // Look up or create an adjacency using the source MAC address.
    let adjacencies = iface.state.lan_adjacencies.get_mut(level);
    let level_usage = level.into();
    let (_, adj) =
        match adjacencies.get_mut_by_snpa(&mut arenas.adjacencies, src) {
            Some((adj_idx, adj)) => {
                if hello.source != adj.system_id {
                    adjacencies.update_system_id(adj_idx, adj, hello.source);
                }
                adj.level_capability = hello.circuit_type;
                adj.level_usage = level_usage;
                (adj_idx, adj)
            }
            None => {
                new_adj = true;
                adjacencies.insert(
                    &mut arenas.adjacencies,
                    src,
                    hello.source,
                    hello.circuit_type,
                    level_usage,
                )
            }
        };

    // Update adjacency with received PDU values.
    let old_priority = adj.priority;
    let old_state = adj.state;
    adj.priority = Some(priority);
    adj.lan_id = Some(lan_id);
    adj.area_addrs = hello.tlvs.area_addrs().cloned().collect();
    adj.neighbors = hello.tlvs.neighbors().cloned().collect();

    // Restart hold timer.
    adj.holdtimer_reset(iface, instance, hello.holdtime);

    // Check for two-way communication.
    let iface_snpa = iface.system.mac_addr.unwrap();
    if adj.neighbors.contains(&iface_snpa) {
        adj.state_change(
            iface,
            instance,
            AdjacencyEvent::HelloTwoWayRcvd,
            AdjacencyState::Up,
        );
    } else {
        adj.state_change(
            iface,
            instance,
            AdjacencyEvent::HelloOneWayRcvd,
            AdjacencyState::Initializing,
        );
    }

    // Restart Hello Tx task if this is a new adjacency (updated list
    // of neighbors).
    if new_adj {
        iface.hello_interval_start(instance, level_usage);
    }

    // Trigger DIS election if priority or state changed.
    if adj.priority != old_priority || adj.state != old_state {
        let msg = DisElectionMsg {
            iface_key: iface.id.into(),
            level,
        };
        let _ = instance.tx.protocol_input.dis_election.send(msg);
    }

    Ok(())
}

pub(crate) fn process_pdu_hello_p2p(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_idx: InterfaceIndex,
    src: [u8; 6],
    hello: Hello,
) -> Result<(), AdjacencyRejectError> {
    let iface = &mut arenas.interfaces[iface_idx];

    // Validate PDU type.
    if iface.config.interface_type != InterfaceType::PointToPoint {
        return Err(AdjacencyRejectError::InvalidHelloType);
    }

    // Check for duplicate System-ID.
    if hello.source == instance.config.system_id.unwrap() {
        return Err(AdjacencyRejectError::DuplicateSystemId);
    }

    // Check for an area match.
    let area_match = hello
        .tlvs
        .area_addrs()
        .any(|addr| instance.config.area_addrs.contains(addr));

    // Process existing or new adjacency.
    let mut adj = match iface.state.p2p_adjacency.take() {
        Some(adj) => {
            // Determine if the PDU can be accepted based on area match and
            // level usage.
            let accept = match (area_match, adj.level_usage) {
                (true, LevelType::L1 | LevelType::L2)
                | (false, LevelType::L2 | LevelType::All) => {
                    adj.level_usage.intersects(hello.circuit_type)
                }
                (true, LevelType::All) => adj.level_usage == hello.circuit_type,
                _ => false,
            };
            if !accept {
                return Err(AdjacencyRejectError::WrongSystem);
            }

            // Reject PDU if the System-ID doesn't match (see IS-IS 8.2.5.2.d).
            if adj.system_id != hello.source {
                return Err(AdjacencyRejectError::WrongSystem);
            }
            adj
        }
        None => {
            // Determine level usage based on area match and circuit type.
            let Some(level_usage) = (match area_match {
                true => {
                    // Area matches: resolve level based on circuit type.
                    iface
                        .config
                        .level_type
                        .resolved
                        .intersection(hello.circuit_type)
                }
                false => {
                    // Non-matching area: only accept L2 circuit type.
                    if hello.circuit_type != LevelType::L1 {
                        Some(LevelType::L2)
                    } else {
                        None
                    }
                }
            }) else {
                return Err(AdjacencyRejectError::WrongSystem);
            };

            // Create a new adjacency.
            Adjacency::new(
                0,
                src,
                hello.source,
                hello.circuit_type,
                level_usage,
            )
        }
    };

    // Update adjacency.
    adj.area_addrs = hello.tlvs.area_addrs().cloned().collect();

    // Restart hold timer.
    adj.holdtimer_reset(iface, instance, hello.holdtime);

    // Transition the adjacency to the "Up" state.
    adj.state_change(
        iface,
        instance,
        AdjacencyEvent::HelloOneWayRcvd,
        AdjacencyState::Up,
    );
    iface.state.p2p_adjacency = Some(adj);

    Ok(())
}

pub(crate) fn process_pdu_lsp(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_idx: InterfaceIndex,
    src: [u8; 6],
    bytes: Bytes,
    mut lsp: Lsp,
) -> Result<(), Error> {
    let iface = &mut arenas.interfaces[iface_idx];
    let system_id = instance.config.system_id.unwrap();

    // Set the level based on the PDU type, and discard the LSP if the level
    // is incompatible with the interface.
    let level = if lsp.hdr.pdu_type == PduType::LspL1 {
        LevelNumber::L1
    } else {
        LevelNumber::L2
    };
    if !iface.config.level_type.resolved.intersects(level) {
        return Ok(());
    }

    // Validate the "Maximum Area Addresses" field.
    if level == LevelNumber::L1
        && lsp.hdr.max_area_addrs != 0
        && lsp.hdr.max_area_addrs != 3
    {
        iface.state.event_counters.max_area_addr_mismatch += 1;
        iface.state.discontinuity_time = Utc::now();
        notification::max_area_addresses_mismatch(
            instance,
            iface,
            lsp.hdr.max_area_addrs,
            &bytes,
        );
        return Ok(());
    }

    // Lookup adjacency.
    let Some(adj) = (match iface.config.interface_type {
        InterfaceType::Broadcast => iface
            .state
            .lan_adjacencies
            .get(level)
            .get_by_snpa(&arenas.adjacencies, src)
            .map(|(_, adj)| adj),
        InterfaceType::PointToPoint => iface
            .state
            .p2p_adjacency
            .as_ref()
            .filter(|adj| adj.level_usage.intersects(level)),
    }) else {
        // Couldn't find a matching adjacency. Discard the LSP.
        return Ok(());
    };

    // Send YANG notification.
    notification::lsp_received(instance, iface, &lsp, &adj.system_id);

    // Check if we're receiving a purge from a self-originated LSP.
    if lsp.rem_lifetime == 0 && lsp.lsp_id.system_id == system_id {
        // Send YANG notification.
        notification::own_lsp_purge(instance, iface, &lsp);

        // Update event counter.
        instance.state.counters.get_mut(level).own_lsp_purge += 1;
        iface.state.discontinuity_time = Utc::now();
    }

    // Validate LSP checksum.
    lsp.raw = bytes;
    if lsp.cksum == 0 {
        // ISO 10589 - Section 7.3.14.2:
        // "A Link State PDU received with a zero checksum shall be treated as
        // if the Remaining Lifetime were zero. The age, if not zero, shall be
        // overwritten with zero".
        lsp.rem_lifetime = 0;
    } else if !lsp.is_checksum_valid() {
        // Send error notification.
        notification::lsp_error_detected(instance, iface, &lsp);

        // Log why the LSP is being discarded.
        Debug::LspDiscard(level, &lsp).log();

        // Discard LSP.
        return Ok(());
    }

    // NOTE: Per RFC 3719, LSPs with a Remaining Lifetime greater than MaxAge
    // should not be discarded as originally specified. MaxAge is now variable
    // and no longer a fixed architectural constant.

    // Lookup LSP in the database.
    let lse = instance
        .state
        .lsdb
        .get(level)
        .get_by_lspid(&arenas.lsp_entries, &lsp.lsp_id)
        .map(|(_, lse)| lse);

    // LSP expiration synchronization (ISO 10589 - Section 7.3.16.4.a).
    if lsp.rem_lifetime == 0 && lse.is_none() {
        if iface.config.interface_type != InterfaceType::Broadcast {
            // Send an acknowledgement.
            let pdu = Pdu::Snp(Snp::new(
                level,
                LanId::from((system_id, iface.state.circuit_id)),
                None,
                SnpTlvs::new([LspEntry {
                    rem_lifetime: lsp.rem_lifetime,
                    lsp_id: lsp.lsp_id,
                    cksum: lsp.cksum,
                    seqno: lsp.seqno,
                }]),
            ));
            iface.enqueue_pdu(pdu, level);
        }
        return Ok(());
    }

    // Check if this is a self-originated LSP.
    if lsp.lsp_id.system_id == system_id {
        if lse.is_none() {
            // Self-originated LSP not found in the LSDB, so it should be purged
            // from the network.
            lsp.set_rem_lifetime(0);
            for iface in arenas.interfaces.iter_mut() {
                iface.srm_list_add(level, lsp.clone());
            }
            return Ok(());
        }

        // Check if the LSP exists in the LSDB and the received LSP is
        // considered more recent.
        if let Some(lse) = lse
            && lsp_compare(&lse.data, lsp.seqno, lsp.rem_lifetime)
                == Ordering::Less
        {
            // Increase LSP sequence number and regenerate.
            let lsp = Lsp::new(
                level,
                instance.config.lsp_lifetime,
                lse.data.lsp_id,
                lsp.seqno + 1,
                lse.data.flags,
                lse.data.tlvs.clone(),
            );
            lsdb::lsp_originate(instance, arenas, level, lsp);
        }

        return Ok(());
    }

    // Compare the LSP in the database (if it exists) to the incoming LSP.
    match lse.map(|lse| lsp_compare(&lse.data, lsp.seqno, lsp.rem_lifetime)) {
        None | Some(Ordering::Less) => {
            // Store the new LSP, replacing any existing one.
            let lse =
                lsdb::install(instance, &mut arenas.lsp_entries, level, lsp);
            let lsp = &lse.data;
            lse.flags.insert(LspEntryFlags::RECEIVED);

            // Update LSP flooding flags for the incoming interface.
            iface.srm_list_del(level, &lsp.lsp_id);
            if iface.config.interface_type != InterfaceType::Broadcast {
                iface.ssn_list_add(level, lsp.as_snp_entry());
            }

            // Update LSP flooding flags for the other interfaces.
            let iface_id = iface.id;
            for other_iface in arenas
                .interfaces
                .iter_mut()
                .filter(|other_iface| other_iface.id != iface_id)
            {
                other_iface.srm_list_add(level, lsp.clone());
                other_iface.ssn_list_del(level, &lsp.lsp_id);
            }
        }
        Some(Ordering::Equal) => {
            let lse = lse.unwrap();

            // LSP confusion handling (ISO 10589 - Section 7.3.16.2).
            if lse.data.cksum != lsp.cksum {
                if lse.flags.contains(LspEntryFlags::RECEIVED) {
                    // Treat it as if its Remaining Lifetime had expired.
                    instance.tx.protocol_input.lsp_purge(
                        level,
                        lse.id,
                        LspPurgeReason::Confusion,
                    );
                } else {
                    // Increase LSP sequence number and regenerate.
                    instance.tx.protocol_input.lsp_refresh(level, lse.id);

                    // Send YANG notification.
                    notification::sequence_number_skipped(
                        instance, iface, &lsp,
                    );

                    // Update event counter.
                    instance.state.counters.get_mut(level).seqno_skipped += 1;
                    iface.state.discontinuity_time = Utc::now();
                }
                return Ok(());
            }

            // Update LSP flooding flags for the incoming interface.
            iface.srm_list_del(level, &lsp.lsp_id);
            if iface.config.interface_type != InterfaceType::Broadcast {
                iface.ssn_list_add(level, lsp.as_snp_entry());
            }
        }
        Some(Ordering::Greater) => {
            // Update LSP flooding flags for the incoming interface.
            let lsp_id = lsp.lsp_id;
            iface.srm_list_add(level, lsp);
            iface.ssn_list_del(level, &lsp_id);
        }
    }

    Ok(())
}

pub(crate) fn process_pdu_snp(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_idx: InterfaceIndex,
    src: [u8; 6],
    bytes: Bytes,
    snp: Snp,
) -> Result<(), Error> {
    let iface = &mut arenas.interfaces[iface_idx];

    // Set the level based on the PDU type, and discard the SNP if the level
    // is incompatible with the interface.
    let level = if matches!(snp.hdr.pdu_type, PduType::CsnpL1 | PduType::PsnpL1)
    {
        LevelNumber::L1
    } else {
        LevelNumber::L2
    };
    if !iface.config.level_type.resolved.intersects(level) {
        return Ok(());
    }

    // Validate the "Maximum Area Addresses" field.
    if level == LevelNumber::L1
        && snp.hdr.max_area_addrs != 0
        && snp.hdr.max_area_addrs != 3
    {
        iface.state.event_counters.max_area_addr_mismatch += 1;
        iface.state.discontinuity_time = Utc::now();
        notification::max_area_addresses_mismatch(
            instance,
            iface,
            snp.hdr.max_area_addrs,
            &bytes,
        );
        return Ok(());
    }

    // Discard PSNP if we're not the DIS for the broadcast interface.
    if iface.config.interface_type == InterfaceType::Broadcast
        && snp.summary.is_none()
        && !iface.is_dis(level)
    {
        return Ok(());
    }

    // Lookup adjacency.
    let Some(_adj) = (match iface.config.interface_type {
        InterfaceType::Broadcast => iface
            .state
            .lan_adjacencies
            .get(level)
            .get_by_snpa(&arenas.adjacencies, src)
            .map(|(_, adj)| adj),
        InterfaceType::PointToPoint => iface
            .state
            .p2p_adjacency
            .as_ref()
            .filter(|adj| adj.level_usage.intersects(level)),
    }) else {
        // Couldn't find a matching adjacency. Discard the SNP.
        return Ok(());
    };

    // Iterate over all LSP entries.
    let lsp_entries = snp
        .tlvs
        .lsp_entries()
        .map(|entry| (entry.lsp_id, *entry))
        .collect::<HashMap<_, _>>();
    for entry in lsp_entries.values() {
        // Lookup LSP in the database.
        let lse = instance
            .state
            .lsdb
            .get(level)
            .get_by_lspid(&arenas.lsp_entries, &entry.lsp_id)
            .map(|(_, lse)| lse);

        // Check if the LSP entry in the received SNP is newer than the
        // corresponding stored LSP and update the LSP flooding flags
        // accordingly.
        if let Some(lse) = lse {
            match lsp_compare(&lse.data, entry.seqno, entry.rem_lifetime) {
                // LSP confusion handling (ISO 10589 - Section 7.3.16.2).
                Ordering::Equal if lse.data.cksum != entry.cksum => {
                    if lse.flags.contains(LspEntryFlags::RECEIVED) {
                        // Treat it as if its Remaining Lifetime had expired.
                        instance.tx.protocol_input.lsp_purge(
                            level,
                            lse.id,
                            LspPurgeReason::Confusion,
                        );
                    } else {
                        // Increase LSP sequence number and regenerate.
                        instance.tx.protocol_input.lsp_refresh(level, lse.id);

                        // Send YANG notification.
                        notification::sequence_number_skipped(
                            instance, iface, &lse.data,
                        );

                        // Update event counter.
                        instance.state.counters.get_mut(level).seqno_skipped +=
                            1;
                        iface.state.discontinuity_time = Utc::now();
                    }
                }
                Ordering::Equal => {
                    iface.srm_list_del(level, &entry.lsp_id);
                }
                Ordering::Greater => {
                    iface.ssn_list_del(level, &entry.lsp_id);
                    iface.srm_list_add(level, lse.data.clone());
                }
                Ordering::Less => {
                    iface.ssn_list_add(level, *entry);
                    iface.srm_list_del(level, &entry.lsp_id);
                }
            }
            continue;
        }

        // ISO 10589 - Section 7.3.15.2.b.5:
        // "If no database entry exists for the LSP, and the reported Remaining
        // Lifetime, Checksum and Sequence Number fields of the LSP are all
        // non-zero, create an entry with sequence number 0".
        if entry.rem_lifetime != 0 && entry.cksum != 0 && entry.seqno != 0 {
            let lsp = Lsp::new(
                level,
                entry.rem_lifetime,
                entry.lsp_id,
                0,
                Default::default(),
                Default::default(),
            );
            let lse =
                lsdb::install(instance, &mut arenas.lsp_entries, level, lsp);
            iface.ssn_list_add(level, lse.data.as_snp_entry());
        }
    }

    // Complete Sequence Numbers PDU processing.
    //
    // Flood LSPs we have that the neighbor doesn't.
    if let Some((start, end)) = snp.summary {
        for lsp in instance
            .state
            .lsdb
            .get_mut(level)
            .range(&arenas.lsp_entries, start..=end)
            .map(|lse| &lse.data)
            .filter(|lsp| !lsp_entries.contains_key(&lsp.lsp_id))
            // Exclude LSPs with zero Remaining Lifetime.
            .filter(|lsp| lsp.rem_lifetime != 0)
            // Exclude LSPs with zero sequence number.
            .filter(|lsp| lsp.seqno != 0)
        {
            iface.srm_list_add(level, lsp.clone());
        }
    }

    Ok(())
}

// ===== Adjacency hold timer expiry =====

pub(crate) fn process_lan_adj_holdtimer_expiry(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_key: InterfaceKey,
    adj_key: AdjacencyKey,
    level: LevelNumber,
) -> Result<(), Error> {
    // Lookup interface.
    let (_iface_idx, iface) = arenas.interfaces.get_mut_by_key(&iface_key)?;

    // Lookup adjacency.
    let (adj_idx, adj) = iface
        .state
        .lan_adjacencies
        .get_mut(level)
        .get_mut_by_key(&mut arenas.adjacencies, &adj_key)?;

    // Delete adjacency.
    adj.state_change(
        iface,
        instance,
        AdjacencyEvent::HoldtimeExpired,
        AdjacencyState::Down,
    );
    iface
        .state
        .lan_adjacencies
        .get_mut(level)
        .delete(&mut arenas.adjacencies, adj_idx);

    // Restart Hello Tx task (updated list of neighbors).
    iface.hello_interval_start(instance, level);

    Ok(())
}

pub(crate) fn process_p2p_adj_holdtimer_expiry(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_key: InterfaceKey,
) -> Result<(), Error> {
    // Lookup interface.
    let (_iface_idx, iface) = arenas.interfaces.get_mut_by_key(&iface_key)?;

    // Delete adjacency.
    if let Some(mut adj) = iface.state.p2p_adjacency.take() {
        adj.state_change(
            iface,
            instance,
            AdjacencyEvent::HoldtimeExpired,
            AdjacencyState::Down,
        );
    }

    Ok(())
}

// ===== DIS election =====

pub(crate) fn process_dis_election(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_key: InterfaceKey,
    level: LevelNumber,
) -> Result<(), Error> {
    // Lookup interface.
    let (_iface_idx, iface) = arenas.interfaces.get_mut_by_key(&iface_key)?;

    // Run DIS election.
    let dis = iface.dis_election(instance, &arenas.adjacencies, level);

    // Return if no DIS change.
    if *iface.state.dis.get(level) == dis {
        return Ok(());
    }

    // Log DIS change.
    Debug::InterfaceDisChange(&iface.name, level, &dis).log();

    // Update DIS.
    let old_dis = std::mem::replace(iface.state.dis.get_mut(level), dis);

    // Update event counter.
    iface.state.event_counters.lan_dis_changes += 1;
    iface.state.discontinuity_time = Utc::now();

    // Restart Hello Tx task.
    iface.hello_interval_start(instance, level);

    // Process DIS changes.
    match (old_dis, dis) {
        (Some(old), _) if old.myself => {
            // We're no longer the DIS.
            iface.dis_stop(instance);
        }
        (_, Some(new)) if new.myself => {
            // We're the new DIS.
            iface.dis_start(instance);
        }
        _ => {}
    }

    // Schedule LSP reorigination.
    instance.schedule_lsp_origination(level);

    Ok(())
}

// ===== Request to send PSNP =====

pub(crate) fn process_send_psnp(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_key: InterfaceKey,
    level: LevelNumber,
) -> Result<(), Error> {
    // Lookup interface.
    let (_iface_idx, iface) = arenas.interfaces.get_mut_by_key(&iface_key)?;

    // Do not send PSNP if we're the DIS.
    if iface.config.interface_type == InterfaceType::Broadcast
        && iface.is_dis(level)
    {
        return Ok(());
    }

    // Do not send empty PSNP.
    if iface.state.ssn_list.get(level).is_empty() {
        return Ok(());
    }

    // Add as many LSP entries that will fit in a single PDU.
    let mut lsp_entries = vec![];
    let mtu = iface.iso_mtu() as usize;
    for _ in 0..SnpTlvs::max_lsp_entries(mtu - Snp::PSNP_HEADER_LEN as usize) {
        if let Some((_, lsp_entry)) =
            iface.state.ssn_list.get_mut(level).pop_first()
        {
            lsp_entries.push(lsp_entry);
        } else {
            break;
        }
    }

    // Generate PDU.
    let pdu = Pdu::Snp(Snp::new(
        level,
        LanId::from((
            instance.config.system_id.unwrap(),
            iface.state.circuit_id,
        )),
        None,
        SnpTlvs::new(lsp_entries),
    ));

    // Enqueue PDU for transmission.
    iface.enqueue_pdu(pdu, level);

    Ok(())
}

// ===== Request to send CSNP =====

pub(crate) fn process_send_csnp(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    iface_key: InterfaceKey,
    level: LevelNumber,
) -> Result<(), Error> {
    // Lookup interface.
    let (_iface_idx, iface) = arenas.interfaces.get_mut_by_key(&iface_key)?;

    // Do not send CSNP if we aren't the DIS.
    if iface.config.interface_type == InterfaceType::Broadcast
        && !iface.is_dis(level)
    {
        return Ok(());
    }

    // Set CSNP source.
    let source = LanId::from((
        instance.config.system_id.unwrap(),
        iface.state.circuit_id,
    ));

    // Calculate maximum of LSP entries per PDU.
    let mtu = iface.iso_mtu() as usize;
    let max_lsp_entries =
        SnpTlvs::max_lsp_entries(mtu - Snp::CSNP_HEADER_LEN as usize);

    // Closure to generate and send CSNP;
    let mut send_csnp = |level, source, start, end, lsp_entries: Vec<_>| {
        // Generate PDU.
        let pdu = Pdu::Snp(Snp::new(
            level,
            source,
            Some((start, end)),
            SnpTlvs::new(lsp_entries),
        ));

        // Enqueue PDU for transmission.
        iface.enqueue_pdu(pdu, level);
    };

    // Iterate over LSDB and send as many CSNPs as necessary.
    let mut start = LspId::from([0; 8]);
    let mut lsp_entries = vec![];
    let mut lsdb_iter = instance
        .state
        .lsdb
        .get(level)
        .iter(&arenas.lsp_entries)
        .map(|lse| &lse.data)
        .peekable();
    while let Some(lsp) = lsdb_iter.next() {
        // Add current LSP entry.
        lsp_entries.push(lsp.as_snp_entry());

        // Check if this is the last LSP.
        let Some(next_lsp) = lsdb_iter.peek() else {
            // Send the final CSNP.
            let end = LspId::from([0xff; 8]);
            (send_csnp)(level, source, start, end, lsp_entries);
            break;
        };

        // If max LSP entries reached, send current CSNP.
        if lsp_entries.len() == max_lsp_entries {
            // Set end LSP ID to current LSP ID.
            let end = lsp.lsp_id;
            let lsp_entries = std::mem::take(&mut lsp_entries);
            (send_csnp)(level, source, start, end, lsp_entries);

            // Update start for the next CSNP.
            start = next_lsp.lsp_id;
        }
    }

    Ok(())
}

// ===== LSP origination event =====

pub(crate) fn process_lsp_originate(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
) -> Result<(), Error> {
    // Clear LSP origination backoff.
    instance.state.lsp_orig_backoff = None;
    let Some(level_type) = instance.state.lsp_orig_pending.take() else {
        return Ok(());
    };

    // Originate LSPs for levels with pending requests.
    for level in [LevelNumber::L1, LevelNumber::L2]
        .into_iter()
        .filter(|level| instance.config.level_type.intersects(level))
        .filter(|level| level_type.intersects(level))
    {
        lsdb::lsp_originate_all(instance, arenas, level);
    }

    Ok(())
}

// ===== LSP purge event =====

pub(crate) fn process_lsp_purge(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    level: LevelNumber,
    lse_key: LspEntryKey,
    reason: LspPurgeReason,
) -> Result<(), Error> {
    // Lookup LSP entry in the LSDB.
    let (_, lse) = instance
        .state
        .lsdb
        .get_mut(level)
        .get_mut_by_key(&mut arenas.lsp_entries, &lse_key)?;
    let lsp = &mut lse.data;

    // Check if the LSP expired.
    let expired = lsp.rem_lifetime() == 0;

    // Log LSP purge.
    Debug::LspPurge(level, lsp, reason).log();

    // Set remaining lifetime to zero if it's not already.
    lsp.set_rem_lifetime(0);

    // Stop the LSP's expiration and refresh timers.
    lse.expiry_timer = None;
    lse.refresh_timer = None;

    // Send purged LSP to all interfaces.
    for iface in arenas.interfaces.iter_mut() {
        iface.srm_list_add(level, lsp.clone());
    }

    // Mark the LSP as purged and start the delete timer.
    lse.flags.insert(LspEntryFlags::PURGED);
    let delete_timeout = if expired {
        lsdb::LSP_ZERO_AGE_LIFETIME
    } else {
        instance.config.lsp_lifetime as u64
    };
    let delete_timer = tasks::lsp_delete_timer(
        level,
        lse.id,
        delete_timeout,
        &instance.tx.protocol_input.lsp_delete,
    );
    lse.delete_timer = Some(delete_timer);

    Ok(())
}

// ===== LSP delete event =====

pub(crate) fn process_lsp_delete(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    level: LevelNumber,
    lse_key: LspEntryKey,
) -> Result<(), Error> {
    // Lookup LSP entry in the LSDB.
    let lsdb = instance.state.lsdb.get_mut(level);
    let (lse_idx, lse) = lsdb.get_by_key(&arenas.lsp_entries, &lse_key)?;
    assert!(lse.flags.contains(LspEntryFlags::PURGED));

    // Log LSP deletion.
    Debug::LspDelete(level, &lse.data).log();

    // Delete the LSP entry from the LSDB.
    lsdb.delete(&mut arenas.lsp_entries, lse_idx);

    Ok(())
}

// ===== LSP refresh event =====

pub(crate) fn process_lsp_refresh(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
    level: LevelNumber,
    lse_key: LspEntryKey,
) -> Result<(), Error> {
    // Lookup LSP entry in the LSDB.
    let lsp = instance
        .state
        .lsdb
        .get(level)
        .get_by_key(&arenas.lsp_entries, &lse_key)
        .map(|(_, lse)| &lse.data)?;

    // Log LSP refresh.
    Debug::LspRefresh(level, lsp).log();

    // Originate new instance of the LSP.
    let lsp = Lsp::new(
        level,
        instance.config.lsp_lifetime,
        lsp.lsp_id,
        lsp.seqno + 1,
        lsp.flags,
        lsp.tlvs.clone(),
    );
    lsdb::lsp_originate(instance, arenas, level, lsp);

    Ok(())
}