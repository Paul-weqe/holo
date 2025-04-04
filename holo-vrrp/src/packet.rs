//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//
// Sponsored by NLnet as part of the Next Generation Internet initiative.
// See: https://nlnet.nl/NGI0
//

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use holo_utils::bytes::{BytesExt, BytesMutExt};
use holo_utils::ip::AddressFamily;
use holo_utils::socket::TTL_MAX;
use internet_checksum::Checksum;
use serde::{Deserialize, Serialize};

use crate::instance::Version;
use crate::network::ICMP_PROTO_NUMBER;

// Type aliases.
pub type DecodeResult<T> = Result<T, DecodeError>;

//
// VRRP v2 Packet Format.
//
//  0                   1                   2                   3
//  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |Version| Type  | Virtual Rtr ID|   Priority    | Count IP Addrs|
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |   Auth Type   |   Adver Int   |          Checksum             |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                         IP Address (1)                        |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                            .                                  |
// |                            .                                  |
// |                            .                                  |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                         IP Address (n)                        |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                     Authentication Data (1)                   |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                     Authentication Data (2)                   |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//
// VRRP v3 Packet Format
// 0                   1                   2                   3
// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |Version| Type  | Virtual Rtr ID|   Priority    |IPvX Addr Count|
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |Reserve| Max Advertise Interval|          Checksum             |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                                                               |
// +                                                               +
// |                       IPvX Address(es)                        |
// +                                                               +
// +                                                               +
// +                                                               +
// +                                                               +
// |                                                               |
// +                                                               +
// |                                                               |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//
#[derive(Clone, Debug, Eq, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct VrrpHdr {
    pub version: Version,
    pub hdr_type: u8,
    pub vrid: u8,
    pub priority: u8,
    pub count_ip: u8,
    pub adver_int: u16,
    pub checksum: u16,
    pub ip_addresses: Vec<IpAddr>,
}

//
// IP packet header
//
//  0                   1                   2                   3
//  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |Version|  IHL  |Type of Service|          Total Length         |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |         Identification        |Flags|      Fragment Offset    |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |  Time to Live |    Protocol   |         Header Checksum       |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                       Source Address                          |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                    Destination Address                        |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                    Options                    |    Padding    |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//
#[derive(Clone, Debug, Eq, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct Ipv4Hdr {
    pub version: u8,
    pub ihl: u8,
    pub tos: u8,
    pub total_length: u16,
    pub identification: u16,
    pub flags: u8,
    pub offset: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src_address: Ipv4Addr,
    pub dst_address: Ipv4Addr,
    pub options: Option<u32>,
    pub padding: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct EthernetHdr {
    pub dst_mac: [u8; 6],
    pub src_mac: [u8; 6],
    pub ethertype: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct ArpHdr {
    pub hw_type: u16,
    pub proto_type: u16,
    pub hw_length: u8,
    pub proto_length: u8,
    pub operation: u16,
    pub sender_hw_address: [u8; 6],
    pub sender_proto_address: Ipv4Addr,
    pub target_hw_address: [u8; 6],
    pub target_proto_address: Ipv4Addr,
}

// Headers for VRRP packets with IPv4 headers.
#[derive(Clone, Debug, Eq, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct Vrrp4Packet {
    pub ip: Ipv4Hdr,
    pub vrrp: VrrpHdr,
}

// Neighbor Advertisement Packet (ICMPV6 + NA fields).
//
//   0                   1                   2                   3
//   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |     Type      |     Code      |          Checksum             |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |R|S|O|                     Reserved                            |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |                                                               |
//  +                                                               +
//  |                                                               |
//  +                       Target Address                          +
//  |                                                               |
//  +                                                               +
//  |                                                               |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |   Options ...
//  +-+-+-+-+-+-+-+-+-+-+-+-
#[derive(Clone, Debug, Eq, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct NeighborAdvertisement {
    pub icmp_type: u8,
    pub code: u8,
    pub checksum: u16,
    pub r: u8,
    pub s: u8,
    pub o: u8,
    pub reserved: u32,
    pub target_address: Ipv6Addr,
}

#[derive(Debug, Eq, PartialEq)]
#[derive(Deserialize, Serialize)]
pub enum DecodeError {
    ChecksumError,
    IncompletePacket,
    PacketLengthError { vrid: u8, version: Version },
    IpTtlError { ttl: u8 },
    VersionError { vrid: u8 },
}

// ===== impl Packet =====

impl VrrpHdr {
    // Minimum number of bytes in a VRRP header (either v2 or v3).
    const MIN_LEN: usize = 8;
    // Valid VRRP protocol versions.
    pub const VALID_VERSIONS: [u8; 2] = [2, 3];
    // Byte offset where the checksum field is located within the VRRP header.
    pub const CHECKSUM_OFFSET: i32 = 6;
    // Maximum number of virtual IP addresses allowed in a VRRP advertisement.
    const MAX_VIRTUAL_IP_COUNT: usize = 20;

    // Encodes VRRP packet into a bytes buffer.
    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::with_capacity(114);
        let ver_type = (self.version.version() << 4) | self.hdr_type;
        buf.put_u8(ver_type);
        buf.put_u8(self.vrid);
        buf.put_u8(self.priority);
        buf.put_u8(self.count_ip);

        match self.version {
            Version::V2 => {
                buf.put_u8(0);
                buf.put_u8(self.adver_int as u8);
                buf.put_u16(self.checksum);
                for addr in &self.ip_addresses {
                    if let IpAddr::V4(ipv4_addr) = addr {
                        buf.put_ipv4(ipv4_addr);
                    }
                }
                buf.put_u32(0);
                buf.put_u32(0);
            }
            Version::V3(_) => {
                buf.put_u16(self.adver_int & 0x0FFF);
                buf.put_u16(self.checksum);
                for addr in &self.ip_addresses {
                    buf.put_ip(addr);
                }
            }
        }

        // Generate checksum.
        if let AddressFamily::Ipv4 = self.version.address_family() {
            let mut check = Checksum::new();
            check.add_bytes(&buf);
            buf[6..8].copy_from_slice(&check.checksum());
        }

        buf
    }

    // Decodes VRRP packet from a bytes buffer.
    pub fn decode(data: &[u8], af: AddressFamily) -> DecodeResult<Self> {
        let pkt_size = data.len();
        if pkt_size < Self::MIN_LEN {
            return Err(DecodeError::IncompletePacket);
        }

        let mut buf: Bytes = Bytes::copy_from_slice(data);
        let ver_type = buf.get_u8();
        let ver = ver_type >> 4;
        let hdr_type = ver_type & 0x0F;
        let vrid = buf.get_u8();
        let version = match (ver, af) {
            (2, AddressFamily::Ipv4) => Version::V2,
            (3, AddressFamily::Ipv4) => Version::V3(AddressFamily::Ipv4),
            (3, AddressFamily::Ipv6) => Version::V3(AddressFamily::Ipv6),
            _ => return Err(DecodeError::VersionError { vrid }),
        };
        if pkt_size > Self::max_length(version) {
            return Err(DecodeError::PacketLengthError { vrid, version });
        }
        let priority = buf.get_u8();
        let count_ip = buf.get_u8();
        let adver_int;
        let checksum;
        let mut ip_addresses = vec![];
        match version {
            Version::V2 => {
                let _auth_type = buf.get_u8();
                adver_int = buf.get_u8() as u16;
                checksum = buf.get_u16();
                for _ in 0..count_ip {
                    ip_addresses.push(IpAddr::V4(buf.get_ipv4()));
                }
                let _auth_data = buf.get_u32();
                let _auth_data2 = buf.get_u32();
            }
            Version::V3(af) => {
                adver_int = buf.get_u16() & 0x0FFF;
                checksum = buf.get_u16();
                for _ in 0..count_ip {
                    match af {
                        AddressFamily::Ipv4 => {
                            ip_addresses.push(IpAddr::V4(buf.get_ipv4()));
                        }
                        AddressFamily::Ipv6 => {
                            ip_addresses.push(IpAddr::V6(buf.get_ipv6()));
                        }
                    }
                }
            }
        }

        // Checksum validation.
        match af {
            AddressFamily::Ipv4 => {
                let mut check = Checksum::new();
                check.add_bytes(data);
                if check.checksum() != [0, 0] {
                    return Err(DecodeError::ChecksumError);
                }
            }
            AddressFamily::Ipv6 => {
                // For IPv6, checksum validation is offloaded to the kernel.
            }
        }

        Ok(Self {
            version,
            hdr_type,
            vrid,
            priority,
            count_ip,
            adver_int,
            checksum,
            ip_addresses,
        })
    }

    // Maximum number of bytes in a packet.
    pub fn max_length(version: Version) -> usize {
        Self::MIN_LEN
            + version.address_family().addr_len() * Self::MAX_VIRTUAL_IP_COUNT
    }
}

impl Ipv4Hdr {
    const MIN_LEN: usize = 20;

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();

        // ver_ihl -> version[4 bits] + ihl[4 bits]
        buf.put_u8((self.version << 4) | self.ihl);
        buf.put_u8(self.tos);
        buf.put_u16(self.total_length);
        buf.put_u16(self.identification);

        // flag_off -> flags[4 bits] + offset[12 bits]
        let flag_off: u16 = ((self.flags as u16) << 12) | self.offset;
        buf.put_u16(flag_off);
        buf.put_u8(self.ttl);
        buf.put_u8(self.protocol);
        buf.put_u16(self.checksum);
        buf.put_ipv4(&self.src_address);
        buf.put_ipv4(&self.dst_address);

        // the header length for IP is between 20 and 24
        // when 24, the options and padding fields are present.
        if let (Some(options), Some(padding)) = (self.options, self.padding) {
            let opt_pad: u32 = (options << 8) | (padding as u32);
            buf.put_u32(opt_pad);
        }

        let mut check = Checksum::new();
        check.add_bytes(&buf);
        buf[10..12].copy_from_slice(&check.checksum());
        buf
    }

    pub fn decode(data: &[u8]) -> DecodeResult<Self> {
        let mut buf = Bytes::copy_from_slice(data);

        // ver_ihl -> version[4 bits] + ihl[4 bits]
        let ver_ihl = buf.get_u8();
        let version = ver_ihl >> 4;
        let ihl = ver_ihl & 0x0F;

        let tos = buf.get_u8();
        let total_length = buf.get_u16();
        let identification = buf.get_u16();

        // flag_off -> flags[4 bits] + offset[12 bits]
        let flag_off = buf.get_u16();
        let flags: u8 = (flag_off >> 12) as u8;
        let offset: u16 = flag_off & 0xFFF;

        let ttl = buf.get_u8();
        if ttl != TTL_MAX {
            return Err(DecodeError::IpTtlError { ttl });
        }
        let protocol = buf.get_u8();
        let checksum = buf.get_u16();

        let src_address = buf.get_ipv4();
        let dst_address = buf.get_ipv4();

        let mut options: Option<u32> = None;
        let mut padding: Option<u8> = None;

        if ihl > Self::MIN_LEN as u8 {
            let opt_pad = buf.get_u32();
            options = Some(opt_pad >> 8);
            padding = Some((opt_pad & 0xFF) as u8);
        }

        let mut check = Checksum::new();
        check.add_bytes(data);
        if check.checksum() != [0, 0] {
            return Err(DecodeError::ChecksumError);
        }

        Ok(Self {
            version,
            ihl,
            tos,
            total_length,
            identification,
            flags,
            offset,
            ttl,
            protocol,
            checksum,
            src_address,
            dst_address,
            options,
            padding,
        })
    }
}

impl EthernetHdr {
    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        self.dst_mac.iter().for_each(|i| buf.put_u8(*i));
        self.src_mac.iter().for_each(|i| buf.put_u8(*i));
        buf.put_u16(self.ethertype);
        buf
    }

    pub fn decode(data: &[u8]) -> DecodeResult<Self> {
        let dst_mac = &data[0..6].try_into();
        let dst_mac: [u8; 6] = dst_mac.unwrap();

        let src_mac = &data[6..12].try_into();
        let src_mac: [u8; 6] = src_mac.unwrap();

        Ok(Self {
            dst_mac,
            src_mac,
            ethertype: libc::ETH_P_IP as _,
        })
    }
}

impl Vrrp4Packet {
    // maximum size of IP + vrrp header.
    const MAX_LEN: usize = 130;

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::with_capacity(Self::MAX_LEN);
        buf.put(self.ip.encode());
        buf.put(self.vrrp.encode());
        buf
    }
}

impl NeighborAdvertisement {
    const PKT_LEN: usize = 192;
    // Number of bytes in the ipv6 pseudo header
    const PSEUDO_LENGTH: usize = 40;
    const PAYLOAD_LENGTH: u32 = 24;

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::with_capacity(Self::PKT_LEN);
        buf.put_u8(self.icmp_type);
        buf.put_u8(self.code);
        buf.put_u16(self.checksum);

        let rso_reserved = ((self.r as u32) << 31)
            | ((self.s as u32) << 30)
            | ((self.o as u32) << 29)
            | ((self.reserved) >> 3);

        buf.put_u32(rso_reserved);
        buf.put_ipv6(&self.target_address);
        buf
    }

    pub fn pseudo_header(&self) -> BytesMut {
        let mut buf = BytesMut::with_capacity(Self::PSEUDO_LENGTH);
        buf.put_ipv6(&self.target_address);
        buf.put_ipv6(&self.target_address);
        buf.put_u32(Self::PAYLOAD_LENGTH);
        buf.put_i32(ICMP_PROTO_NUMBER);
        buf
    }
}

impl ArpHdr {
    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::with_capacity(28);
        buf.put_u16(self.hw_type);
        buf.put_u16(self.proto_type);
        buf.put_u8(self.hw_length);
        buf.put_u8(self.proto_length);
        buf.put_u16(self.operation);
        buf.put_slice(&self.sender_hw_address);
        buf.put_ipv4(&self.sender_proto_address);
        buf.put_slice(&self.target_hw_address);
        buf.put_ipv4(&self.target_proto_address);
        buf
    }

    pub fn decode(data: &[u8]) -> DecodeResult<Self> {
        let mut buf = Bytes::copy_from_slice(data);
        let mut sender_hw_address: [u8; 6] = [0; 6];
        let mut target_hw_address: [u8; 6] = [0; 6];

        let hw_type = buf.get_u16();
        let proto_type = buf.get_u16();
        let hw_length = buf.get_u8();
        let proto_length = buf.get_u8();
        let operation = buf.get_u16();
        buf.copy_to_slice(&mut sender_hw_address);
        let sender_proto_address = buf.get_ipv4();
        buf.copy_to_slice(&mut target_hw_address);
        let target_proto_address = buf.get_ipv4();

        Ok(Self {
            hw_type,
            proto_type,
            hw_length,
            proto_length,
            operation,
            sender_hw_address,
            sender_proto_address,
            target_hw_address,
            target_proto_address,
        })
    }
}
