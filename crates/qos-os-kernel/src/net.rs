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
use alloc::collections::{BTreeMap, VecDeque};
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
    let src_port = udp.src_port_val();
    let dst_port = udp.dst_port_val();
    let payload = &data[8..];
    
    crate::serial_println!(
        "[NET] UDP {}:{} -> :{} len={}",
        src_ip, src_port, dst_port, udp.length_val()
    );
    
    // Check for DHCP response (from port 67 to port 68)
    if src_port == 67 && dst_port == 68 {
        process_dhcp(payload);
        return;
    }
    
    // Deliver to bound socket
    udp_deliver(src_ip, src_port, dst_port, payload);
}

/// Process TCP packet
fn process_tcp(data: &[u8], ip_hdr: &Ipv4Header) {
    if data.len() < 20 {
        return;
    }
    
    let tcp = unsafe { &*(data.as_ptr() as *const TcpHeader) };
    let src_ip = Ipv4Addr(ip_hdr.src);
    let src_port = tcp.src_port_val();
    let dst_port = tcp.dst_port_val();
    let flags = tcp.flags();
    let seq = tcp.seq();
    let ack = tcp.ack();
    
    let mut flag_str = alloc::string::String::new();
    if flags & tcp_flags::SYN != 0 { flag_str.push_str("SYN "); }
    if flags & tcp_flags::ACK != 0 { flag_str.push_str("ACK "); }
    if flags & tcp_flags::FIN != 0 { flag_str.push_str("FIN "); }
    if flags & tcp_flags::RST != 0 { flag_str.push_str("RST "); }
    if flags & tcp_flags::PSH != 0 { flag_str.push_str("PSH "); }
    
    crate::serial_println!(
        "[NET] TCP {}:{} -> :{} [{}] seq={} ack={}",
        src_ip, src_port, dst_port,
        flag_str.trim(), seq, ack
    );
    
    // Get data offset and payload
    let data_offset = (tcp.data_offset() as usize) * 4;
    let payload = if data.len() > data_offset {
        &data[data_offset..]
    } else {
        &[]
    };
    
    // Deliver to TCP state machine
    tcp_deliver(src_ip, src_port, dst_port, payload, flags, seq, ack);
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

// =============================================================================
// UDP SOCKET IMPLEMENTATION
// =============================================================================

/// UDP socket state
#[derive(Debug, Clone)]
pub struct UdpSocket {
    pub local_port: u16,
    pub remote_addr: Option<(Ipv4Addr, u16)>,
}

/// UDP socket handles
static UDP_SOCKETS: Mutex<BTreeMap<u16, UdpSocket>> = Mutex::new(BTreeMap::new());

/// Queue for received UDP datagrams per port
static UDP_RX_QUEUE: Mutex<BTreeMap<u16, VecDeque<UdpDatagram>>> = Mutex::new(BTreeMap::new());

/// Received UDP datagram
#[derive(Debug, Clone)]
pub struct UdpDatagram {
    pub src_ip: Ipv4Addr,
    pub src_port: u16,
    pub data: Vec<u8>,
}

impl UdpSocket {
    /// Create and bind a new UDP socket
    pub fn bind(port: u16) -> Result<Self, &'static str> {
        let mut sockets = UDP_SOCKETS.lock();
        
        if sockets.contains_key(&port) {
            return Err("port already in use");
        }
        
        let socket = UdpSocket {
            local_port: port,
            remote_addr: None,
        };
        
        sockets.insert(port, socket.clone());
        UDP_RX_QUEUE.lock().insert(port, VecDeque::new());
        
        crate::serial_println!("[UDP] Socket bound to port {}", port);
        Ok(socket)
    }
    
    /// Create a new UDP socket with ephemeral port
    pub fn new() -> Self {
        let port = alloc_port();
        Self::bind(port).unwrap()
    }
    
    /// Connect to remote address (optional, sets default destination)
    pub fn connect(&mut self, addr: Ipv4Addr, port: u16) {
        self.remote_addr = Some((addr, port));
        crate::serial_println!("[UDP] Socket connected to {}:{}", addr, port);
    }
    
    /// Send datagram to specified address
    pub fn send_to(&self, data: &[u8], dst_ip: Ipv4Addr, dst_port: u16) -> Result<usize, &'static str> {
        if !crate::e1000::is_available() {
            return Err("network device not available");
        }
        
        let packet = create_udp(dst_ip, dst_port, self.local_port, data);
        crate::e1000::send(&packet)?;
        
        crate::serial_println!("[UDP] Sent {} bytes to {}:{}", data.len(), dst_ip, dst_port);
        Ok(data.len())
    }
    
    /// Send datagram to connected address
    pub fn send(&self, data: &[u8]) -> Result<usize, &'static str> {
        match self.remote_addr {
            Some((ip, port)) => self.send_to(data, ip, port),
            None => Err("socket not connected"),
        }
    }
    
    /// Receive datagram (non-blocking)
    pub fn recv_from(&self) -> Option<UdpDatagram> {
        // Poll for new packets first
        crate::e1000::poll();
        
        // Check receive queue
        UDP_RX_QUEUE.lock()
            .get_mut(&self.local_port)
            .and_then(|q| q.pop_front())
    }
    
    /// Receive with timeout (simple polling with spin)
    pub fn recv_timeout(&self, timeout_ms: u32) -> Option<UdpDatagram> {
        let start = crate::timer::uptime_ms();
        
        loop {
            if let Some(dgram) = self.recv_from() {
                return Some(dgram);
            }
            
            if crate::timer::uptime_ms() - start > timeout_ms as u64 {
                return None;
            }
            
            // Small delay
            for _ in 0..10000 {
                core::hint::spin_loop();
            }
        }
    }
    
    /// Close the socket
    pub fn close(self) {
        UDP_SOCKETS.lock().remove(&self.local_port);
        UDP_RX_QUEUE.lock().remove(&self.local_port);
        crate::serial_println!("[UDP] Socket on port {} closed", self.local_port);
    }
}

/// Deliver received UDP packet to socket
pub fn udp_deliver(src_ip: Ipv4Addr, src_port: u16, dst_port: u16, data: &[u8]) {
    let mut queues = UDP_RX_QUEUE.lock();
    
    if let Some(queue) = queues.get_mut(&dst_port) {
        let datagram = UdpDatagram {
            src_ip,
            src_port,
            data: data.to_vec(),
        };
        queue.push_back(datagram);
        crate::serial_println!("[UDP] Delivered {} bytes to port {}", data.len(), dst_port);
    }
}

/// Get list of bound UDP ports (for netstat)
pub fn udp_ports() -> Vec<u16> {
    UDP_SOCKETS.lock().keys().copied().collect()
}

// =============================================================================
// TCP SOCKET IMPLEMENTATION (Basic)
// =============================================================================

/// TCP connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

/// TCP socket
#[derive(Debug, Clone)]
pub struct TcpSocket {
    pub local_port: u16,
    pub remote_addr: Option<(Ipv4Addr, u16)>,
    pub state: TcpState,
    pub seq_num: u32,
    pub ack_num: u32,
}

/// TCP socket handles
static TCP_SOCKETS: Mutex<BTreeMap<u16, TcpSocket>> = Mutex::new(BTreeMap::new());

/// TCP receive queue per port
static TCP_RX_QUEUE: Mutex<BTreeMap<u16, VecDeque<Vec<u8>>>> = Mutex::new(BTreeMap::new());

impl TcpSocket {
    /// Create and bind a TCP socket (for listening)
    pub fn bind(port: u16) -> Result<Self, &'static str> {
        let mut sockets = TCP_SOCKETS.lock();
        
        if sockets.contains_key(&port) {
            return Err("port already in use");
        }
        
        let socket = TcpSocket {
            local_port: port,
            remote_addr: None,
            state: TcpState::Closed,
            seq_num: 0x1000, // Initial sequence number
            ack_num: 0,
        };
        
        sockets.insert(port, socket.clone());
        TCP_RX_QUEUE.lock().insert(port, VecDeque::new());
        
        crate::serial_println!("[TCP] Socket bound to port {}", port);
        Ok(socket)
    }
    
    /// Start listening for connections
    pub fn listen(&mut self) -> Result<(), &'static str> {
        if self.state != TcpState::Closed {
            return Err("socket not in closed state");
        }
        self.state = TcpState::Listen;
        
        // Update in global map
        if let Some(sock) = TCP_SOCKETS.lock().get_mut(&self.local_port) {
            sock.state = TcpState::Listen;
        }
        
        crate::serial_println!("[TCP] Listening on port {}", self.local_port);
        Ok(())
    }
    
    /// Connect to remote host (active open)
    pub fn connect(&mut self, dst_ip: Ipv4Addr, dst_port: u16) -> Result<(), &'static str> {
        if !crate::e1000::is_available() {
            return Err("network device not available");
        }
        
        self.remote_addr = Some((dst_ip, dst_port));
        self.state = TcpState::SynSent;
        
        // Send SYN
        let packet = create_tcp_packet(
            dst_ip, dst_port, self.local_port,
            self.seq_num, 0,
            tcp_flags::SYN,
            &[]
        );
        crate::e1000::send(&packet)?;
        
        crate::serial_println!("[TCP] SYN sent to {}:{}", dst_ip, dst_port);
        
        // Update global state
        if let Some(sock) = TCP_SOCKETS.lock().get_mut(&self.local_port) {
            sock.state = TcpState::SynSent;
            sock.remote_addr = Some((dst_ip, dst_port));
        }
        
        Ok(())
    }
    
    /// Send data on established connection
    pub fn send(&mut self, data: &[u8]) -> Result<usize, &'static str> {
        if self.state != TcpState::Established {
            return Err("connection not established");
        }
        
        let (dst_ip, dst_port) = self.remote_addr.ok_or("no remote address")?;
        
        let packet = create_tcp_packet(
            dst_ip, dst_port, self.local_port,
            self.seq_num, self.ack_num,
            tcp_flags::PSH | tcp_flags::ACK,
            data
        );
        
        crate::e1000::send(&packet)?;
        self.seq_num = self.seq_num.wrapping_add(data.len() as u32);
        
        crate::serial_println!("[TCP] Sent {} bytes to {}:{}", data.len(), dst_ip, dst_port);
        Ok(data.len())
    }
    
    /// Receive data (non-blocking)
    pub fn recv(&self) -> Option<Vec<u8>> {
        crate::e1000::poll();
        TCP_RX_QUEUE.lock()
            .get_mut(&self.local_port)
            .and_then(|q| q.pop_front())
    }
    
    /// Close the connection
    pub fn close(&mut self) -> Result<(), &'static str> {
        if self.state == TcpState::Established {
            if let Some((dst_ip, dst_port)) = self.remote_addr {
                let packet = create_tcp_packet(
                    dst_ip, dst_port, self.local_port,
                    self.seq_num, self.ack_num,
                    tcp_flags::FIN | tcp_flags::ACK,
                    &[]
                );
                let _ = crate::e1000::send(&packet);
                self.state = TcpState::FinWait1;
            }
        }
        
        TCP_SOCKETS.lock().remove(&self.local_port);
        TCP_RX_QUEUE.lock().remove(&self.local_port);
        
        crate::serial_println!("[TCP] Socket on port {} closed", self.local_port);
        Ok(())
    }
}

/// Get a copy of TCP socket state by port
pub fn tcp_get_socket(port: u16) -> Option<TcpSocket> {
    TCP_SOCKETS.lock().get(&port).cloned()
}

/// Create a TCP packet
pub fn create_tcp_packet(
    dst_ip: Ipv4Addr,
    dst_port: u16,
    src_port: u16,
    seq: u32,
    ack: u32,
    flags: u16,
    data: &[u8]
) -> Vec<u8> {
    let config = NET_CONFIG.lock();
    
    // TCP header (20 bytes minimum)
    let data_offset = 5u8; // 5 * 4 = 20 bytes
    let mut tcp = Vec::with_capacity(20 + data.len());
    tcp.extend_from_slice(&src_port.to_be_bytes());
    tcp.extend_from_slice(&dst_port.to_be_bytes());
    tcp.extend_from_slice(&seq.to_be_bytes());
    tcp.extend_from_slice(&ack.to_be_bytes());
    tcp.push(data_offset << 4); // Data offset + reserved
    tcp.push((flags & 0x3F) as u8); // Flags
    tcp.extend_from_slice(&8192u16.to_be_bytes()); // Window size
    tcp.extend_from_slice(&[0, 0]); // Checksum placeholder
    tcp.extend_from_slice(&[0, 0]); // Urgent pointer
    tcp.extend_from_slice(data);
    
    // TCP pseudo-header for checksum
    let tcp_len = tcp.len() as u16;
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(&config.ip.0);
    pseudo.extend_from_slice(&dst_ip.0);
    pseudo.push(0);
    pseudo.push(ip_proto::TCP);
    pseudo.extend_from_slice(&tcp_len.to_be_bytes());
    pseudo.extend_from_slice(&tcp);
    
    let tcp_csum = ip_checksum(&pseudo);
    tcp[16..18].copy_from_slice(&tcp_csum.to_be_bytes());
    
    // IP header
    let total_len = 20 + tcp.len();
    let mut ip_hdr = vec![0x45, 0x00];
    ip_hdr.extend_from_slice(&(total_len as u16).to_be_bytes());
    ip_hdr.extend_from_slice(&[0x00, 0x00]);
    ip_hdr.extend_from_slice(&[0x40, 0x00]);
    ip_hdr.push(64);
    ip_hdr.push(ip_proto::TCP);
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
    packet.extend_from_slice(&tcp);
    
    packet
}

/// Deliver TCP data to socket
pub fn tcp_deliver(src_ip: Ipv4Addr, src_port: u16, dst_port: u16, data: &[u8], flags: u16, seq: u32, ack: u32) {
    let mut sockets = TCP_SOCKETS.lock();
    
    if let Some(sock) = sockets.get_mut(&dst_port) {
        match sock.state {
            TcpState::Listen if flags & tcp_flags::SYN != 0 => {
                // Incoming connection - send SYN-ACK
                sock.remote_addr = Some((src_ip, src_port));
                sock.ack_num = seq.wrapping_add(1);
                sock.state = TcpState::SynReceived;
                
                let packet = create_tcp_packet(
                    src_ip, src_port, dst_port,
                    sock.seq_num, sock.ack_num,
                    tcp_flags::SYN | tcp_flags::ACK,
                    &[]
                );
                let _ = crate::e1000::send(&packet);
                crate::serial_println!("[TCP] SYN-ACK sent to {}:{}", src_ip, src_port);
            }
            TcpState::SynSent if flags & (tcp_flags::SYN | tcp_flags::ACK) != 0 => {
                // Received SYN-ACK, send ACK
                sock.ack_num = seq.wrapping_add(1);
                sock.seq_num = sock.seq_num.wrapping_add(1);
                sock.state = TcpState::Established;
                
                let packet = create_tcp_packet(
                    src_ip, src_port, dst_port,
                    sock.seq_num, sock.ack_num,
                    tcp_flags::ACK,
                    &[]
                );
                let _ = crate::e1000::send(&packet);
                crate::serial_println!("[TCP] Connection established to {}:{}", src_ip, src_port);
            }
            TcpState::SynReceived if flags & tcp_flags::ACK != 0 => {
                sock.state = TcpState::Established;
                crate::serial_println!("[TCP] Connection established from {}:{}", src_ip, src_port);
            }
            TcpState::Established => {
                if flags & tcp_flags::FIN != 0 {
                    // Peer wants to close
                    sock.ack_num = seq.wrapping_add(1);
                    sock.state = TcpState::CloseWait;
                    
                    let packet = create_tcp_packet(
                        src_ip, src_port, dst_port,
                        sock.seq_num, sock.ack_num,
                        tcp_flags::ACK,
                        &[]
                    );
                    let _ = crate::e1000::send(&packet);
                } else if !data.is_empty() {
                    // Data received
                    sock.ack_num = seq.wrapping_add(data.len() as u32);
                    
                    // Send ACK
                    let packet = create_tcp_packet(
                        src_ip, src_port, dst_port,
                        sock.seq_num, sock.ack_num,
                        tcp_flags::ACK,
                        &[]
                    );
                    let _ = crate::e1000::send(&packet);
                    
                    // Deliver to receive queue
                    drop(sockets);
                    if let Some(queue) = TCP_RX_QUEUE.lock().get_mut(&dst_port) {
                        queue.push_back(data.to_vec());
                    }
                    crate::serial_println!("[TCP] Received {} bytes on port {}", data.len(), dst_port);
                    return;
                }
            }
            _ => {}
        }
    }
}

/// Get list of TCP sockets (for netstat)
pub fn tcp_sockets() -> Vec<(u16, TcpState, Option<(Ipv4Addr, u16)>)> {
    TCP_SOCKETS.lock()
        .iter()
        .map(|(port, sock)| (*port, sock.state, sock.remote_addr))
        .collect()
}

// =============================================================================
// DHCP CLIENT
// =============================================================================

/// DHCP message types
mod dhcp_msg {
    pub const DISCOVER: u8 = 1;
    pub const OFFER: u8 = 2;
    pub const REQUEST: u8 = 3;
    pub const ACK: u8 = 5;
}

/// DHCP options
mod dhcp_opt {
    pub const SUBNET_MASK: u8 = 1;
    pub const ROUTER: u8 = 3;
    pub const DNS: u8 = 6;
    pub const REQUESTED_IP: u8 = 50;
    pub const MESSAGE_TYPE: u8 = 53;
    pub const SERVER_ID: u8 = 54;
    pub const END: u8 = 255;
}

/// DHCP transaction ID
static DHCP_XID: AtomicU32 = AtomicU32::new(0x12345678);

use core::sync::atomic::AtomicU32;

/// Create DHCP Discover packet
pub fn create_dhcp_discover() -> Vec<u8> {
    let config = NET_CONFIG.lock();
    let xid = DHCP_XID.load(Ordering::SeqCst);
    
    let mut dhcp = vec![0u8; 240];
    dhcp[0] = 1;  // BOOTREQUEST
    dhcp[1] = 1;  // Ethernet
    dhcp[2] = 6;  // Hardware addr len
    dhcp[3] = 0;  // Hops
    dhcp[4..8].copy_from_slice(&xid.to_be_bytes());  // Transaction ID
    dhcp[8..10].copy_from_slice(&0u16.to_be_bytes()); // Seconds
    dhcp[10..12].copy_from_slice(&0x8000u16.to_be_bytes()); // Flags (broadcast)
    // Client IP, Your IP, Server IP, Gateway IP = 0
    dhcp[28..34].copy_from_slice(&config.mac.0); // Client MAC
    
    // Magic cookie
    dhcp[236..240].copy_from_slice(&[99, 130, 83, 99]);
    
    // DHCP options
    dhcp.push(dhcp_opt::MESSAGE_TYPE);
    dhcp.push(1);
    dhcp.push(dhcp_msg::DISCOVER);
    
    dhcp.push(dhcp_opt::END);
    
    // Pad to minimum size
    while dhcp.len() < 300 {
        dhcp.push(0);
    }
    
    // Wrap in UDP (src 68, dst 67)
    create_udp(Ipv4Addr::BROADCAST, 67, 68, &dhcp)
}

/// Create DHCP Request packet
pub fn create_dhcp_request(offered_ip: Ipv4Addr, server_ip: Ipv4Addr) -> Vec<u8> {
    let config = NET_CONFIG.lock();
    let xid = DHCP_XID.load(Ordering::SeqCst);
    
    let mut dhcp = vec![0u8; 240];
    dhcp[0] = 1;  // BOOTREQUEST
    dhcp[1] = 1;  // Ethernet
    dhcp[2] = 6;  // Hardware addr len
    dhcp[3] = 0;  // Hops
    dhcp[4..8].copy_from_slice(&xid.to_be_bytes());
    dhcp[8..10].copy_from_slice(&0u16.to_be_bytes());
    dhcp[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    dhcp[28..34].copy_from_slice(&config.mac.0);
    
    // Magic cookie
    dhcp[236..240].copy_from_slice(&[99, 130, 83, 99]);
    
    // DHCP options
    dhcp.push(dhcp_opt::MESSAGE_TYPE);
    dhcp.push(1);
    dhcp.push(dhcp_msg::REQUEST);
    
    dhcp.push(dhcp_opt::REQUESTED_IP);
    dhcp.push(4);
    dhcp.extend_from_slice(&offered_ip.0);
    
    dhcp.push(dhcp_opt::SERVER_ID);
    dhcp.push(4);
    dhcp.extend_from_slice(&server_ip.0);
    
    dhcp.push(dhcp_opt::END);
    
    while dhcp.len() < 300 {
        dhcp.push(0);
    }
    
    create_udp(Ipv4Addr::BROADCAST, 67, 68, &dhcp)
}

/// DHCP client state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpState {
    Init,
    Selecting,
    Requesting,
    Bound,
}

static DHCP_STATE: Mutex<DhcpState> = Mutex::new(DhcpState::Init);
static DHCP_OFFERED_IP: Mutex<Option<Ipv4Addr>> = Mutex::new(None);
static DHCP_SERVER_IP: Mutex<Option<Ipv4Addr>> = Mutex::new(None);

/// Start DHCP discovery
pub fn dhcp_discover() -> Result<(), &'static str> {
    if !crate::e1000::is_available() {
        return Err("network device not available");
    }
    
    // Generate new transaction ID
    let xid = crate::rtc::unix_time() as u32;
    DHCP_XID.store(xid, Ordering::SeqCst);
    
    *DHCP_STATE.lock() = DhcpState::Selecting;
    
    let packet = create_dhcp_discover();
    crate::e1000::send(&packet)?;
    
    crate::serial_println!("[DHCP] DISCOVER sent (xid={:08x})", xid);
    Ok(())
}

/// Process DHCP response (called from UDP processing)
pub fn process_dhcp(data: &[u8]) {
    if data.len() < 240 {
        return;
    }
    
    // Check magic cookie
    if &data[236..240] != &[99, 130, 83, 99] {
        return;
    }
    
    // Check transaction ID
    let xid = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    if xid != DHCP_XID.load(Ordering::SeqCst) {
        return;
    }
    
    let your_ip = Ipv4Addr([data[16], data[17], data[18], data[19]]);
    let server_ip = Ipv4Addr([data[20], data[21], data[22], data[23]]);
    
    // Parse options
    let mut i = 240;
    let mut msg_type = 0u8;
    let mut subnet = Ipv4Addr::ZERO;
    let mut router = Ipv4Addr::ZERO;
    let mut dns = Ipv4Addr::ZERO;
    
    while i < data.len() && data[i] != dhcp_opt::END {
        if data[i] == 0 {
            i += 1;
            continue;
        }
        
        let opt = data[i];
        let len = data.get(i + 1).copied().unwrap_or(0) as usize;
        let val = &data[i + 2..i + 2 + len.min(data.len() - i - 2)];
        
        match opt {
            dhcp_opt::MESSAGE_TYPE if len >= 1 => msg_type = val[0],
            dhcp_opt::SUBNET_MASK if len >= 4 => subnet = Ipv4Addr([val[0], val[1], val[2], val[3]]),
            dhcp_opt::ROUTER if len >= 4 => router = Ipv4Addr([val[0], val[1], val[2], val[3]]),
            dhcp_opt::DNS if len >= 4 => dns = Ipv4Addr([val[0], val[1], val[2], val[3]]),
            _ => {}
        }
        
        i += 2 + len;
    }
    
    let state = *DHCP_STATE.lock();
    
    match (state, msg_type) {
        (DhcpState::Selecting, dhcp_msg::OFFER) => {
            crate::serial_println!("[DHCP] OFFER received: IP={}", your_ip);
            *DHCP_OFFERED_IP.lock() = Some(your_ip);
            *DHCP_SERVER_IP.lock() = Some(server_ip);
            *DHCP_STATE.lock() = DhcpState::Requesting;
            
            // Send REQUEST
            let packet = create_dhcp_request(your_ip, server_ip);
            let _ = crate::e1000::send(&packet);
            crate::serial_println!("[DHCP] REQUEST sent for {}", your_ip);
        }
        (DhcpState::Requesting, dhcp_msg::ACK) => {
            crate::serial_println!("[DHCP] ACK received - IP assigned: {}", your_ip);
            
            // Configure network
            {
                let mut config = NET_CONFIG.lock();
                config.ip = your_ip;
                if subnet != Ipv4Addr::ZERO {
                    config.netmask = subnet;
                }
                if router != Ipv4Addr::ZERO {
                    config.gateway = router;
                }
                if dns != Ipv4Addr::ZERO {
                    config.dns = dns;
                }
            }
            
            *DHCP_STATE.lock() = DhcpState::Bound;
            crate::println!("[DHCP] Network configured:");
            crate::println!("  IP:      {}", your_ip);
            crate::println!("  Netmask: {}", subnet);
            crate::println!("  Gateway: {}", router);
            crate::println!("  DNS:     {}", dns);
        }
        _ => {}
    }
}

/// Get DHCP state
pub fn dhcp_state() -> DhcpState {
    *DHCP_STATE.lock()
}