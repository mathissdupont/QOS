//! Network Stack for QOS
//!
//! Basic TCP/IP networking with:
//! - Ethernet frame handling
//! - ARP (Address Resolution Protocol)
//! - IPv4
//! - ICMP (ping)
//! - UDP
//! - TCP (basic)

use alloc::vec;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use spin::Mutex;
use core::sync::atomic::{AtomicU16, Ordering};

/// MAC address (6 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    pub const BROADCAST: Self = Self([0xFF; 6]);
    pub const ZERO: Self = Self([0; 6]);
    
    pub fn is_broadcast(&self) -> bool {
        self.0 == [0xFF; 6]
    }
}

impl core::fmt::Display for MacAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5])
    }
}

/// IPv4 address
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const ZERO: Self = Self([0; 4]);
    pub const BROADCAST: Self = Self([255, 255, 255, 255]);
    pub const LOCALHOST: Self = Self([127, 0, 0, 1]);
    
    pub fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }
    
    pub fn to_u32(&self) -> u32 {
        u32::from_be_bytes(self.0)
    }
    
    pub fn from_u32(v: u32) -> Self {
        Self(v.to_be_bytes())
    }
}

impl core::fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

/// Ethernet types
pub mod eth_type {
    pub const IPV4: u16 = 0x0800;
    pub const ARP: u16 = 0x0806;
    pub const IPV6: u16 = 0x86DD;
}

/// IP protocols
pub mod ip_proto {
    pub const ICMP: u8 = 1;
    pub const TCP: u8 = 6;
    pub const UDP: u8 = 17;
}

/// Ethernet frame header
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct EthHeader {
    pub dst: [u8; 6],
    pub src: [u8; 6],
    pub eth_type: [u8; 2],
}

impl EthHeader {
    pub fn eth_type_val(&self) -> u16 {
        u16::from_be_bytes(self.eth_type)
    }
}

/// ARP packet
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ArpPacket {
    pub htype: [u8; 2],      // Hardware type (1 for Ethernet)
    pub ptype: [u8; 2],      // Protocol type (0x0800 for IPv4)
    pub hlen: u8,            // Hardware address length (6)
    pub plen: u8,            // Protocol address length (4)
    pub oper: [u8; 2],       // Operation (1=request, 2=reply)
    pub sha: [u8; 6],        // Sender hardware address
    pub spa: [u8; 4],        // Sender protocol address
    pub tha: [u8; 6],        // Target hardware address
    pub tpa: [u8; 4],        // Target protocol address
}

impl ArpPacket {
    pub fn operation(&self) -> u16 {
        u16::from_be_bytes(self.oper)
    }
}

/// IPv4 header
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct Ipv4Header {
    pub version_ihl: u8,
    pub dscp_ecn: u8,
    pub total_length: [u8; 2],
    pub identification: [u8; 2],
    pub flags_fragment: [u8; 2],
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: [u8; 2],
    pub src: [u8; 4],
    pub dst: [u8; 4],
}

impl Ipv4Header {
    pub fn version(&self) -> u8 {
        self.version_ihl >> 4
    }
    
    pub fn ihl(&self) -> u8 {
        self.version_ihl & 0x0F
    }
    
    pub fn header_len(&self) -> usize {
        (self.ihl() as usize) * 4
    }
    
    pub fn total_len(&self) -> u16 {
        u16::from_be_bytes(self.total_length)
    }
}

/// ICMP header
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct IcmpHeader {
    pub icmp_type: u8,
    pub code: u8,
    pub checksum: [u8; 2],
    pub rest: [u8; 4],
}

/// UDP header
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct UdpHeader {
    pub src_port: [u8; 2],
    pub dst_port: [u8; 2],
    pub length: [u8; 2],
    pub checksum: [u8; 2],
}

impl UdpHeader {
    pub fn src_port_val(&self) -> u16 {
        u16::from_be_bytes(self.src_port)
    }
    
    pub fn dst_port_val(&self) -> u16 {
        u16::from_be_bytes(self.dst_port)
    }
    
    pub fn length_val(&self) -> u16 {
        u16::from_be_bytes(self.length)
    }
}

/// TCP header (basic fields)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TcpHeader {
    pub src_port: [u8; 2],
    pub dst_port: [u8; 2],
    pub seq_num: [u8; 4],
    pub ack_num: [u8; 4],
    pub data_offset_flags: [u8; 2],
    pub window: [u8; 2],
    pub checksum: [u8; 2],
    pub urgent_ptr: [u8; 2],
}

impl TcpHeader {
    pub fn src_port_val(&self) -> u16 {
        u16::from_be_bytes(self.src_port)
    }
    
    pub fn dst_port_val(&self) -> u16 {
        u16::from_be_bytes(self.dst_port)
    }
    
    pub fn seq(&self) -> u32 {
        u32::from_be_bytes(self.seq_num)
    }
    
    pub fn ack(&self) -> u32 {
        u32::from_be_bytes(self.ack_num)
    }
    
    pub fn data_offset(&self) -> u8 {
        (self.data_offset_flags[0] >> 4) * 4
    }
    
    pub fn flags(&self) -> u16 {
        u16::from_be_bytes(self.data_offset_flags) & 0x01FF
    }
}

/// TCP flags
pub mod tcp_flags {
    pub const FIN: u16 = 0x001;
    pub const SYN: u16 = 0x002;
    pub const RST: u16 = 0x004;
    pub const PSH: u16 = 0x008;
    pub const ACK: u16 = 0x010;
    pub const URG: u16 = 0x020;
}

/// Network interface configuration
#[derive(Debug, Clone)]
pub struct NetConfig {
    pub mac: MacAddr,
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub dns: Ipv4Addr,
}

impl NetConfig {
    pub fn new() -> Self {
        Self {
            mac: MacAddr([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]), // QEMU default
            ip: Ipv4Addr::new(10, 0, 2, 15),                    // QEMU user mode
            netmask: Ipv4Addr::new(255, 255, 255, 0),
            gateway: Ipv4Addr::new(10, 0, 2, 2),
            dns: Ipv4Addr::new(10, 0, 2, 3),
        }
    }
}

impl Default for NetConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// ARP cache
static ARP_CACHE: Mutex<BTreeMap<Ipv4Addr, MacAddr>> = Mutex::new(BTreeMap::new());

/// Network configuration
static NET_CONFIG: Mutex<NetConfig> = Mutex::new(NetConfig {
    mac: MacAddr([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]),
    ip: Ipv4Addr([10, 0, 2, 15]),
    netmask: Ipv4Addr([255, 255, 255, 0]),
    gateway: Ipv4Addr([10, 0, 2, 2]),
    dns: Ipv4Addr([10, 0, 2, 3]),
});

/// Next port for ephemeral allocation
static NEXT_PORT: AtomicU16 = AtomicU16::new(49152);

/// Calculate IP checksum
pub fn ip_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    
    while i < data.len() - 1 {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    
    !(sum as u16)
}

/// Initialize networking
pub fn init() {
    // Add localhost to ARP cache
    ARP_CACHE.lock().insert(Ipv4Addr::LOCALHOST, MacAddr::ZERO);
    
    crate::serial_println!("[NET] Network stack initialized");
    
    let config = NET_CONFIG.lock();
    crate::serial_println!("[NET] MAC: {}", config.mac);
    crate::serial_println!("[NET] IP:  {}", config.ip);
}

/// Get network configuration
pub fn config() -> NetConfig {
    NET_CONFIG.lock().clone()
}

/// Set IP address
pub fn set_ip(ip: Ipv4Addr) {
    NET_CONFIG.lock().ip = ip;
}

/// Set MAC address
pub fn set_mac(mac: MacAddr) {
    NET_CONFIG.lock().mac = mac;
}

/// Allocate ephemeral port
pub fn alloc_port() -> u16 {
    let port = NEXT_PORT.fetch_add(1, Ordering::SeqCst);
    if port == 65535 {
        NEXT_PORT.store(49152, Ordering::SeqCst);
    }
    port
}

/// Look up MAC address in ARP cache
pub fn arp_lookup(ip: Ipv4Addr) -> Option<MacAddr> {
    ARP_CACHE.lock().get(&ip).copied()
}

/// Add entry to ARP cache
pub fn arp_add(ip: Ipv4Addr, mac: MacAddr) {
    ARP_CACHE.lock().insert(ip, mac);
}

/// Create ARP request packet
pub fn create_arp_request(target_ip: Ipv4Addr) -> Vec<u8> {
    let config = NET_CONFIG.lock();
    let mut packet = Vec::with_capacity(42);
    
    // Ethernet header
    packet.extend_from_slice(&MacAddr::BROADCAST.0); // Destination
    packet.extend_from_slice(&config.mac.0);         // Source
    packet.extend_from_slice(&eth_type::ARP.to_be_bytes());
    
    // ARP
    packet.extend_from_slice(&1u16.to_be_bytes());   // Hardware type (Ethernet)
    packet.extend_from_slice(&0x0800u16.to_be_bytes()); // Protocol (IPv4)
    packet.push(6);                                  // Hardware addr len
    packet.push(4);                                  // Protocol addr len
    packet.extend_from_slice(&1u16.to_be_bytes());   // Operation (request)
    packet.extend_from_slice(&config.mac.0);         // Sender MAC
    packet.extend_from_slice(&config.ip.0);          // Sender IP
    packet.extend_from_slice(&[0; 6]);               // Target MAC (unknown)
    packet.extend_from_slice(&target_ip.0);          // Target IP
    
    packet
}

/// Create ICMP echo request (ping)
pub fn create_ping(dst: Ipv4Addr, seq: u16) -> Vec<u8> {
    let config = NET_CONFIG.lock();
    
    // ICMP payload (timestamp)
    let timestamp = crate::rtc::unix_time();
    let payload: [u8; 8] = timestamp.to_ne_bytes();
    
    // ICMP header + payload
    let mut icmp = Vec::new();
    icmp.push(8);  // Type: Echo Request
    icmp.push(0);  // Code
    icmp.extend_from_slice(&[0, 0]); // Checksum placeholder
    icmp.extend_from_slice(&1u16.to_be_bytes()); // Identifier
    icmp.extend_from_slice(&seq.to_be_bytes()); // Sequence
    icmp.extend_from_slice(&payload);
    
    // Calculate ICMP checksum
    let csum = ip_checksum(&icmp);
    icmp[2..4].copy_from_slice(&csum.to_be_bytes());
    
    // IP header
    let total_len = 20 + icmp.len();
    let mut ip_hdr = vec![
        0x45, // Version (4) + IHL (5)
        0x00, // DSCP + ECN
    ];
    ip_hdr.extend_from_slice(&(total_len as u16).to_be_bytes());
    ip_hdr.extend_from_slice(&[0x00, 0x00]); // Identification
    ip_hdr.extend_from_slice(&[0x40, 0x00]); // Don't fragment
    ip_hdr.push(64); // TTL
    ip_hdr.push(ip_proto::ICMP);
    ip_hdr.extend_from_slice(&[0, 0]); // Checksum placeholder
    ip_hdr.extend_from_slice(&config.ip.0);
    ip_hdr.extend_from_slice(&dst.0);
    
    let hdr_csum = ip_checksum(&ip_hdr);
    ip_hdr[10..12].copy_from_slice(&hdr_csum.to_be_bytes());
    
    // Ethernet header
    let dst_mac = arp_lookup(dst).unwrap_or(MacAddr::BROADCAST);
    let mut packet = Vec::with_capacity(14 + ip_hdr.len() + icmp.len());
    packet.extend_from_slice(&dst_mac.0);
    packet.extend_from_slice(&config.mac.0);
    packet.extend_from_slice(&eth_type::IPV4.to_be_bytes());
    packet.extend_from_slice(&ip_hdr);
    packet.extend_from_slice(&icmp);
    
    packet
}

/// Create UDP packet
pub fn create_udp(dst_ip: Ipv4Addr, dst_port: u16, src_port: u16, data: &[u8]) -> Vec<u8> {
    let config = NET_CONFIG.lock();
    
    // UDP header
    let udp_len = 8 + data.len();
    let mut udp = Vec::with_capacity(udp_len);
    udp.extend_from_slice(&src_port.to_be_bytes());
    udp.extend_from_slice(&dst_port.to_be_bytes());
    udp.extend_from_slice(&(udp_len as u16).to_be_bytes());
    udp.extend_from_slice(&[0, 0]); // Checksum (optional for UDP)
    udp.extend_from_slice(data);
    
    // IP header
    let total_len = 20 + udp.len();
    let mut ip_hdr = vec![0x45, 0x00];
    ip_hdr.extend_from_slice(&(total_len as u16).to_be_bytes());
    ip_hdr.extend_from_slice(&[0x00, 0x00]);
    ip_hdr.extend_from_slice(&[0x40, 0x00]);
    ip_hdr.push(64);
    ip_hdr.push(ip_proto::UDP);
    ip_hdr.extend_from_slice(&[0, 0]);
    ip_hdr.extend_from_slice(&config.ip.0);
    ip_hdr.extend_from_slice(&dst_ip.0);
    
    let hdr_csum = ip_checksum(&ip_hdr);
    ip_hdr[10..12].copy_from_slice(&hdr_csum.to_be_bytes());
    
    // Ethernet
    let dst_mac = arp_lookup(dst_ip).unwrap_or(MacAddr::BROADCAST);
    let mut packet = Vec::new();
    packet.extend_from_slice(&dst_mac.0);
    packet.extend_from_slice(&config.mac.0);
    packet.extend_from_slice(&eth_type::IPV4.to_be_bytes());
    packet.extend_from_slice(&ip_hdr);
    packet.extend_from_slice(&udp);
    
    packet
}

/// Process received Ethernet frame
pub fn process_frame(frame: &[u8]) {
    if frame.len() < 14 {
        return;
    }
    
    let eth_type = u16::from_be_bytes([frame[12], frame[13]]);
    let payload = &frame[14..];
    
    match eth_type {
        eth_type::ARP => process_arp(payload),
        eth_type::IPV4 => process_ipv4(payload),
        _ => {}
    }
}

/// Process ARP packet
fn process_arp(data: &[u8]) {
    if data.len() < 28 {
        return;
    }
    
    let arp = unsafe { &*(data.as_ptr() as *const ArpPacket) };
    let sender_ip = Ipv4Addr(arp.spa);
    let sender_mac = MacAddr(arp.sha);
    
    // Update ARP cache
    arp_add(sender_ip, sender_mac);
    
    // If it's a request for our IP, send reply
    let config = NET_CONFIG.lock();
    if arp.operation() == 1 && Ipv4Addr(arp.tpa) == config.ip {
        crate::serial_println!("[NET] ARP request for our IP from {}", sender_ip);
        // TODO: Send ARP reply
    }
}

/// Process IPv4 packet
fn process_ipv4(data: &[u8]) {
    if data.len() < 20 {
        return;
    }
    
    let ip_hdr = unsafe { &*(data.as_ptr() as *const Ipv4Header) };
    
    if ip_hdr.version() != 4 {
        return;
    }
    
    let header_len = ip_hdr.header_len();
    let payload = &data[header_len..];
    
    match ip_hdr.protocol {
        ip_proto::ICMP => process_icmp(payload, ip_hdr),
        ip_proto::UDP => process_udp(payload, ip_hdr),
        ip_proto::TCP => process_tcp(payload, ip_hdr),
        _ => {}
    }
}

/// Process ICMP packet
fn process_icmp(data: &[u8], ip_hdr: &Ipv4Header) {
    if data.len() < 8 {
        return;
    }
    
    let icmp = unsafe { &*(data.as_ptr() as *const IcmpHeader) };
    let src_ip = Ipv4Addr(ip_hdr.src);
    
    match icmp.icmp_type {
        0 => { // Echo Reply
            crate::serial_println!("[NET] ICMP Echo Reply from {}", src_ip);
        }
        8 => { // Echo Request
            crate::serial_println!("[NET] ICMP Echo Request from {}", src_ip);
            // TODO: Send echo reply
        }
        _ => {}
    }
}

/// Process UDP packet
fn process_udp(data: &[u8], ip_hdr: &Ipv4Header) {
    if data.len() < 8 {
        return;
    }
    
    let udp = unsafe { &*(data.as_ptr() as *const UdpHeader) };
    let src_ip = Ipv4Addr(ip_hdr.src);
    
    crate::serial_println!(
        "[NET] UDP {}:{} -> :{} len={}",
        src_ip, udp.src_port_val(), udp.dst_port_val(), udp.length_val()
    );
}

/// Process TCP packet
fn process_tcp(data: &[u8], ip_hdr: &Ipv4Header) {
    if data.len() < 20 {
        return;
    }
    
    let tcp = unsafe { &*(data.as_ptr() as *const TcpHeader) };
    let src_ip = Ipv4Addr(ip_hdr.src);
    let flags = tcp.flags();
    
    let mut flag_str = alloc::string::String::new();
    if flags & tcp_flags::SYN != 0 { flag_str.push_str("SYN "); }
    if flags & tcp_flags::ACK != 0 { flag_str.push_str("ACK "); }
    if flags & tcp_flags::FIN != 0 { flag_str.push_str("FIN "); }
    if flags & tcp_flags::RST != 0 { flag_str.push_str("RST "); }
    if flags & tcp_flags::PSH != 0 { flag_str.push_str("PSH "); }
    
    crate::serial_println!(
        "[NET] TCP {}:{} -> :{} [{}] seq={} ack={}",
        src_ip, tcp.src_port_val(), tcp.dst_port_val(),
        flag_str.trim(), tcp.seq(), tcp.ack()
    );
}

/// Display network info
pub fn show_info() {
    let config = NET_CONFIG.lock();
    
    crate::println!("Network Configuration:");
    crate::println!("  MAC Address: {}", config.mac);
    crate::println!("  IP Address:  {}", config.ip);
    crate::println!("  Netmask:     {}", config.netmask);
    crate::println!("  Gateway:     {}", config.gateway);
    crate::println!("  DNS Server:  {}", config.dns);
    
    crate::println!("\nARP Cache:");
    let cache = ARP_CACHE.lock();
    if cache.is_empty() {
        crate::println!("  (empty)");
    } else {
        for (ip, mac) in cache.iter() {
            crate::println!("  {} -> {}", ip, mac);
        }
    }
}
