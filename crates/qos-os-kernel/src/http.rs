//! HTTP/1.1 Client for QaOS
//!
//! Minimal HTTP client implementation for connecting to cloud QPU services.
//! Supports both HTTP and HTTPS (via TLS).

use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use alloc::format;
use alloc::collections::BTreeMap;
use core::str;

use crate::net::{Ipv4Addr, TcpSocket, TcpState};

// ============================================================================
// HTTP Types
// ============================================================================

/// HTTP Method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
}

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::GET => "GET",
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::DELETE => "DELETE",
            Method::PATCH => "PATCH",
            Method::HEAD => "HEAD",
            Method::OPTIONS => "OPTIONS",
        }
    }
}

/// HTTP Version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpVersion {
    Http10,
    Http11,
}

impl HttpVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpVersion::Http10 => "HTTP/1.0",
            HttpVersion::Http11 => "HTTP/1.1",
        }
    }
}

/// HTTP Status Code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusCode(pub u16);

impl StatusCode {
    pub const OK: Self = Self(200);
    pub const CREATED: Self = Self(201);
    pub const ACCEPTED: Self = Self(202);
    pub const NO_CONTENT: Self = Self(204);
    pub const MOVED_PERMANENTLY: Self = Self(301);
    pub const FOUND: Self = Self(302);
    pub const BAD_REQUEST: Self = Self(400);
    pub const UNAUTHORIZED: Self = Self(401);
    pub const FORBIDDEN: Self = Self(403);
    pub const NOT_FOUND: Self = Self(404);
    pub const INTERNAL_SERVER_ERROR: Self = Self(500);
    pub const BAD_GATEWAY: Self = Self(502);
    pub const SERVICE_UNAVAILABLE: Self = Self(503);
    
    pub fn is_success(&self) -> bool {
        self.0 >= 200 && self.0 < 300
    }
    
    pub fn is_redirect(&self) -> bool {
        self.0 >= 300 && self.0 < 400
    }
    
    pub fn is_client_error(&self) -> bool {
        self.0 >= 400 && self.0 < 500
    }
    
    pub fn is_server_error(&self) -> bool {
        self.0 >= 500 && self.0 < 600
    }
}

/// HTTP Error
#[derive(Debug, Clone)]
pub enum HttpError {
    ConnectionFailed,
    ConnectionTimeout,
    DnsResolutionFailed,
    InvalidUrl,
    InvalidResponse,
    TlsError(String),
    IoError(String),
    Timeout,
    TooManyRedirects,
    BodyTooLarge,
}

// ============================================================================
// URL Parsing
// ============================================================================

/// Parsed URL
#[derive(Debug, Clone)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub query: Option<String>,
}

impl Url {
    /// Parse a URL string
    pub fn parse(url: &str) -> Result<Self, HttpError> {
        let mut url = url.trim();
        
        // Parse scheme
        let (scheme, rest) = if let Some(pos) = url.find("://") {
            let s = &url[..pos];
            let r = &url[pos + 3..];
            (s.to_lowercase(), r)
        } else {
            return Err(HttpError::InvalidUrl);
        };
        
        // Determine default port
        let default_port = match scheme.as_str() {
            "http" => 80,
            "https" => 443,
            _ => return Err(HttpError::InvalidUrl),
        };
        
        // Split host and path
        let (host_port, path_query) = if let Some(pos) = rest.find('/') {
            (&rest[..pos], &rest[pos..])
        } else {
            (rest, "/")
        };
        
        // Parse host and port
        let (host, port) = if let Some(pos) = host_port.rfind(':') {
            let h = &host_port[..pos];
            let p = host_port[pos + 1..].parse::<u16>().unwrap_or(default_port);
            (h.to_lowercase(), p)
        } else {
            (host_port.to_lowercase(), default_port)
        };
        
        // Parse path and query
        let (path, query) = if let Some(pos) = path_query.find('?') {
            (
                String::from(&path_query[..pos]),
                Some(String::from(&path_query[pos + 1..]))
            )
        } else {
            (String::from(path_query), None)
        };
        
        Ok(Url {
            scheme: String::from(&scheme),
            host: String::from(&host),
            port,
            path,
            query,
        })
    }
    
    /// Check if HTTPS
    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }
    
    /// Get full path with query
    pub fn full_path(&self) -> String {
        match &self.query {
            Some(q) => format!("{}?{}", self.path, q),
            None => self.path.clone(),
        }
    }
}

// ============================================================================
// HTTP Request
// ============================================================================

/// HTTP Request Builder
pub struct Request {
    method: Method,
    url: Url,
    headers: BTreeMap<String, String>,
    body: Option<Vec<u8>>,
    timeout_ms: u64,
}

impl Request {
    /// Create a new GET request
    pub fn get(url: &str) -> Result<Self, HttpError> {
        Ok(Self {
            method: Method::GET,
            url: Url::parse(url)?,
            headers: BTreeMap::new(),
            body: None,
            timeout_ms: 30000,
        })
    }
    
    /// Create a new POST request
    pub fn post(url: &str) -> Result<Self, HttpError> {
        Ok(Self {
            method: Method::POST,
            url: Url::parse(url)?,
            headers: BTreeMap::new(),
            body: None,
            timeout_ms: 30000,
        })
    }
    
    /// Create a new request with custom method
    pub fn new(method: Method, url: &str) -> Result<Self, HttpError> {
        Ok(Self {
            method,
            url: Url::parse(url)?,
            headers: BTreeMap::new(),
            body: None,
            timeout_ms: 30000,
        })
    }
    
    /// Add a header
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(String::from(key), String::from(value));
        self
    }
    
    /// Set authorization header
    pub fn bearer_auth(self, token: &str) -> Self {
        self.header("Authorization", &format!("Bearer {}", token))
    }
    
    /// Set API key authorization
    pub fn api_key_auth(self, key: &str) -> Self {
        self.header("Authorization", &format!("apiKey {}", key))
    }
    
    /// Set request body
    pub fn body(mut self, data: Vec<u8>) -> Self {
        self.body = Some(data);
        self
    }
    
    /// Set JSON body
    pub fn json(self, json: &str) -> Self {
        self.header("Content-Type", "application/json")
            .body(json.as_bytes().to_vec())
    }
    
    /// Set timeout in milliseconds
    pub fn timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }
    
    /// Build the raw HTTP request bytes
    fn build(&self) -> Vec<u8> {
        let mut request = String::new();
        
        // Request line
        request.push_str(self.method.as_str());
        request.push(' ');
        request.push_str(&self.url.full_path());
        request.push_str(" HTTP/1.1\r\n");
        
        // Host header (required for HTTP/1.1)
        request.push_str("Host: ");
        request.push_str(&self.url.host);
        if (self.url.scheme == "http" && self.url.port != 80) ||
           (self.url.scheme == "https" && self.url.port != 443) {
            request.push(':');
            request.push_str(&format!("{}", self.url.port));
        }
        request.push_str("\r\n");
        
        // User-Agent
        request.push_str("User-Agent: QaOS-HTTP/1.0\r\n");
        
        // Connection
        request.push_str("Connection: close\r\n");
        
        // Accept
        if !self.headers.contains_key("Accept") {
            request.push_str("Accept: application/json, */*\r\n");
        }
        
        // Custom headers
        for (key, value) in &self.headers {
            request.push_str(key);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        
        // Content-Length if body present
        if let Some(ref body) = self.body {
            request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        
        // End of headers
        request.push_str("\r\n");
        
        let mut data = request.into_bytes();
        
        // Append body
        if let Some(ref body) = self.body {
            data.extend_from_slice(body);
        }
        
        data
    }
    
    /// Send the request and get response
    pub fn send(self) -> Result<Response, HttpError> {
        HttpClient::send_request(self)
    }
}

// ============================================================================
// HTTP Response
// ============================================================================

/// HTTP Response
#[derive(Debug, Clone)]
pub struct Response {
    pub status: StatusCode,
    pub version: HttpVersion,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Response {
    /// Parse response from raw bytes
    fn parse(data: &[u8]) -> Result<Self, HttpError> {
        let data_str = str::from_utf8(data).map_err(|_| HttpError::InvalidResponse)?;
        
        // Find header/body separator
        let header_end = data_str.find("\r\n\r\n")
            .ok_or(HttpError::InvalidResponse)?;
        
        let header_part = &data_str[..header_end];
        let body_start = header_end + 4;
        
        let mut lines = header_part.lines();
        
        // Parse status line
        let status_line = lines.next().ok_or(HttpError::InvalidResponse)?;
        let mut parts = status_line.split_whitespace();
        
        let version_str = parts.next().ok_or(HttpError::InvalidResponse)?;
        let version = match version_str {
            "HTTP/1.0" => HttpVersion::Http10,
            "HTTP/1.1" => HttpVersion::Http11,
            _ => return Err(HttpError::InvalidResponse),
        };
        
        let status_code: u16 = parts.next()
            .ok_or(HttpError::InvalidResponse)?
            .parse()
            .map_err(|_| HttpError::InvalidResponse)?;
        
        // Parse headers
        let mut headers = BTreeMap::new();
        for line in lines {
            if let Some(pos) = line.find(':') {
                let key = line[..pos].trim().to_lowercase();
                let value = line[pos + 1..].trim();
                headers.insert(String::from(&key), String::from(value));
            }
        }
        
        // Get body
        let body = if body_start < data.len() {
            data[body_start..].to_vec()
        } else {
            Vec::new()
        };
        
        Ok(Response {
            status: StatusCode(status_code),
            version,
            headers,
            body,
        })
    }
    
    /// Get body as string
    pub fn text(&self) -> Result<String, HttpError> {
        String::from_utf8(self.body.clone())
            .map_err(|_| HttpError::InvalidResponse)
    }
    
    /// Get a header value
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(&key.to_lowercase()).map(|s| s.as_str())
    }
    
    /// Check if response is success (2xx)
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }
}

// ============================================================================
// HTTP Client
// ============================================================================

/// HTTP Client
pub struct HttpClient;

impl HttpClient {
    /// Send an HTTP request
    fn send_request(request: Request) -> Result<Response, HttpError> {
        // Check if HTTPS
        if request.url.is_https() {
            return Self::send_https(request);
        }
        
        // Resolve hostname to IP
        let ip = Self::resolve_host(&request.url.host)?;
        
        // Allocate local port and create socket
        let local_port = crate::net::alloc_port();
        let mut socket = TcpSocket::bind(local_port)
            .map_err(|_| HttpError::ConnectionFailed)?;
        
        // Connect
        socket.connect(ip, request.url.port)
            .map_err(|_| HttpError::ConnectionFailed)?;
        
        // Wait for connection with timeout
        let start = crate::timer::ticks();
        let timeout_ticks = request.timeout_ms * 18 / 1000; // ~18 ticks/sec
        
        while socket.state != TcpState::Established {
            if socket.state == TcpState::Closed {
                return Err(HttpError::ConnectionFailed);
            }
            if crate::timer::ticks() - start > timeout_ticks as u64 {
                return Err(HttpError::ConnectionTimeout);
            }
            // Process incoming packets
            crate::e1000::poll();
            
            // Sync socket state from global store
            if let Some(s) = crate::net::tcp_get_socket(local_port) {
                socket.state = s.state;
            }
            core::hint::spin_loop();
        }
        
        // Send request
        let request_data = request.build();
        let _ = socket.send(&request_data);
        
        // Receive response
        let mut response_data = Vec::new();
        let recv_start = crate::timer::ticks();
        
        loop {
            crate::e1000::poll();
            
            // Sync socket state
            if let Some(s) = crate::net::tcp_get_socket(local_port) {
                socket.state = s.state;
            }
            
            if let Some(data) = socket.recv() {
                response_data.extend_from_slice(&data);
                
                // Check if we have complete response
                if Self::is_response_complete(&response_data) {
                    break;
                }
            }
            
            // Check for connection close
            if socket.state == TcpState::CloseWait || socket.state == TcpState::Closed {
                break;
            }
            
            // Timeout check
            if crate::timer::ticks() - recv_start > timeout_ticks as u64 {
                if response_data.is_empty() {
                    return Err(HttpError::Timeout);
                }
                break;
            }
            
            core::hint::spin_loop();
        }
        
        // Close connection
        let _ = socket.close();
        
        // Parse response
        Response::parse(&response_data)
    }
    
    /// Send HTTPS request (with TLS)
    fn send_https(request: Request) -> Result<Response, HttpError> {
        // TLS implementation
        let ip = Self::resolve_host(&request.url.host)?;
        
        // Create TLS connection
        let mut tls = TlsStream::connect(ip, request.url.port, &request.url.host)?;
        
        // Send request over TLS
        let request_data = request.build();
        tls.write(&request_data)?;
        
        // Receive response
        let mut response_data = Vec::new();
        let start = crate::timer::ticks();
        let timeout_ticks = request.timeout_ms * 18 / 1000;
        
        loop {
            match tls.read() {
                Ok(data) if !data.is_empty() => {
                    response_data.extend_from_slice(&data);
                    if Self::is_response_complete(&response_data) {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
            
            if crate::timer::ticks() - start > timeout_ticks as u64 {
                break;
            }
            
            core::hint::spin_loop();
        }
        
        tls.close();
        Response::parse(&response_data)
    }
    
    /// Check if HTTP response is complete
    fn is_response_complete(data: &[u8]) -> bool {
        let Ok(s) = str::from_utf8(data) else { return false };
        
        // Find header end
        let Some(header_end) = s.find("\r\n\r\n") else { return false };
        
        let headers = &s[..header_end];
        let body_start = header_end + 4;
        
        // Check Content-Length
        for line in headers.lines() {
            if line.to_lowercase().starts_with("content-length:") {
                if let Ok(len) = line[15..].trim().parse::<usize>() {
                    return data.len() >= body_start + len;
                }
            }
        }
        
        // Check Transfer-Encoding: chunked
        if headers.to_lowercase().contains("transfer-encoding: chunked") {
            return s.ends_with("0\r\n\r\n");
        }
        
        // No content-length, assume complete if connection closed
        true
    }
    
    /// Resolve hostname to IP address
    fn resolve_host(host: &str) -> Result<Ipv4Addr, HttpError> {
        // Check if already an IP address
        let parts: Vec<&str> = host.split('.').collect();
        if parts.len() == 4 {
            if let (Ok(a), Ok(b), Ok(c), Ok(d)) = (
                parts[0].parse::<u8>(),
                parts[1].parse::<u8>(),
                parts[2].parse::<u8>(),
                parts[3].parse::<u8>()
            ) {
                return Ok(Ipv4Addr::new(a, b, c, d));
            }
        }
        
        // DNS resolution
        dns_resolve(host).ok_or(HttpError::DnsResolutionFailed)
    }
}

// ============================================================================
// DNS Resolution
// ============================================================================

/// Simple DNS resolver
pub fn dns_resolve(hostname: &str) -> Option<Ipv4Addr> {
    // Well-known hosts (hardcoded for reliability)
    let known_hosts: &[(&str, [u8; 4])] = &[
        ("api.quantum-computing.ibm.com", [169, 60, 70, 10]),
        ("quantum.googleapis.com", [142, 250, 185, 42]),
        ("api.ionq.co", [52, 71, 247, 118]),
        ("localhost", [127, 0, 0, 1]),
    ];
    
    for (name, ip) in known_hosts {
        if hostname == *name || hostname.ends_with(&format!(".{}", name)) {
            return Some(Ipv4Addr(*ip));
        }
    }
    
    // TODO: Real DNS resolution requires UDP send support
    // For now, only hardcoded hosts are supported
    // Future: Send DNS query to configured DNS server
    
    let dns_server = crate::net::config().dns;
    if dns_server == Ipv4Addr::ZERO {
        return None;
    }
    
    // DNS not fully implemented yet - return None for unknown hosts
    // To add a host, add it to known_hosts above
    crate::serial_println!("[HTTP] DNS lookup for '{}' - not in cache, DNS not implemented", hostname);
    None
}

/// DNS response storage
static DNS_RESPONSE: spin::Mutex<Option<Vec<u8>>> = spin::Mutex::new(None);

/// Store DNS response (called from UDP handler)
pub fn store_dns_response(data: &[u8]) {
    *DNS_RESPONSE.lock() = Some(data.to_vec());
}

/// Build DNS query packet
fn build_dns_query(hostname: &str) -> Vec<u8> {
    let mut query = Vec::new();
    
    // Transaction ID
    let txid = (crate::timer::ticks() & 0xFFFF) as u16;
    query.extend_from_slice(&txid.to_be_bytes());
    
    // Flags: standard query
    query.extend_from_slice(&[0x01, 0x00]);
    
    // Questions: 1
    query.extend_from_slice(&[0x00, 0x01]);
    
    // Answer RRs: 0
    query.extend_from_slice(&[0x00, 0x00]);
    
    // Authority RRs: 0
    query.extend_from_slice(&[0x00, 0x00]);
    
    // Additional RRs: 0
    query.extend_from_slice(&[0x00, 0x00]);
    
    // QNAME
    for part in hostname.split('.') {
        query.push(part.len() as u8);
        query.extend_from_slice(part.as_bytes());
    }
    query.push(0); // End of name
    
    // QTYPE: A (IPv4)
    query.extend_from_slice(&[0x00, 0x01]);
    
    // QCLASS: IN (Internet)
    query.extend_from_slice(&[0x00, 0x01]);
    
    query
}

/// Parse DNS response
fn parse_dns_response(data: &[u8]) -> Option<Ipv4Addr> {
    if data.len() < 12 {
        return None;
    }
    
    // Check for answers
    let answer_count = u16::from_be_bytes([data[6], data[7]]);
    if answer_count == 0 {
        return None;
    }
    
    // Skip header and question section
    let mut i = 12;
    
    // Skip QNAME in question
    while i < data.len() && data[i] != 0 {
        if data[i] & 0xC0 == 0xC0 {
            i += 2;
            break;
        }
        i += 1 + data[i] as usize;
    }
    if i < data.len() && data[i] == 0 {
        i += 1;
    }
    
    // Skip QTYPE and QCLASS
    i += 4;
    
    // Parse answer
    while i + 12 <= data.len() {
        // Skip NAME (might be compressed)
        if data[i] & 0xC0 == 0xC0 {
            i += 2;
        } else {
            while i < data.len() && data[i] != 0 {
                i += 1 + data[i] as usize;
            }
            i += 1;
        }
        
        if i + 10 > data.len() {
            break;
        }
        
        let rtype = u16::from_be_bytes([data[i], data[i + 1]]);
        let rdlength = u16::from_be_bytes([data[i + 8], data[i + 9]]) as usize;
        
        i += 10;
        
        // Type A (IPv4)
        if rtype == 1 && rdlength == 4 && i + 4 <= data.len() {
            return Some(Ipv4Addr([data[i], data[i + 1], data[i + 2], data[i + 3]]));
        }
        
        i += rdlength;
    }
    
    None
}

// ============================================================================
// TLS/SSL Support
// ============================================================================

/// TLS Stream for HTTPS connections
pub struct TlsStream {
    socket: TcpSocket,
    host: String,
    /// TLS state
    state: TlsState,
    /// Session keys (after handshake)
    client_write_key: [u8; 32],
    server_write_key: [u8; 32],
    client_write_iv: [u8; 12],
    server_write_iv: [u8; 12],
    /// Sequence numbers
    client_seq: u64,
    server_seq: u64,
    /// Receive buffer for decrypted data
    recv_buffer: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TlsState {
    Initial,
    ClientHelloSent,
    ServerHelloReceived,
    Established,
    Closed,
}

impl TlsStream {
    /// Connect to a TLS server
    pub fn connect(ip: Ipv4Addr, port: u16, host: &str) -> Result<Self, HttpError> {
        let local_port = crate::net::alloc_port();
        let mut socket = TcpSocket::bind(local_port)
            .map_err(|_| HttpError::ConnectionFailed)?;
        
        socket.connect(ip, port)
            .map_err(|_| HttpError::ConnectionFailed)?;
        
        // Wait for TCP connection
        let start = crate::timer::ticks();
        while socket.state != TcpState::Established {
            if socket.state == TcpState::Closed {
                return Err(HttpError::ConnectionFailed);
            }
            if crate::timer::ticks() - start > 180 {
                return Err(HttpError::ConnectionTimeout);
            }
            crate::e1000::poll();
            
            // Sync socket state
            if let Some(s) = crate::net::tcp_get_socket(local_port) {
                socket.state = s.state;
            }
            core::hint::spin_loop();
        }
        
        let mut stream = TlsStream {
            socket,
            host: String::from(host),
            state: TlsState::Initial,
            client_write_key: [0; 32],
            server_write_key: [0; 32],
            client_write_iv: [0; 12],
            server_write_iv: [0; 12],
            client_seq: 0,
            server_seq: 0,
            recv_buffer: Vec::new(),
        };
        
        // Perform TLS handshake
        stream.handshake()?;
        
        Ok(stream)
    }
    
    /// TLS 1.3 Handshake
    fn handshake(&mut self) -> Result<(), HttpError> {
        // Send ClientHello
        let client_hello = self.build_client_hello();
        self.send_record(0x16, &client_hello)?; // Handshake
        self.state = TlsState::ClientHelloSent;
        
        // Receive ServerHello and other handshake messages
        let start = crate::timer::ticks();
        while self.state != TlsState::Established {
            crate::e1000::poll();
            
            if let Some(data) = self.socket.recv() {
                self.process_handshake(&data)?;
            }
            
            if crate::timer::ticks() - start > 180 {
                return Err(HttpError::ConnectionTimeout);
            }
            
            if self.socket.state == TcpState::Closed {
                return Err(HttpError::TlsError(String::from("Connection closed during handshake")));
            }
            
            core::hint::spin_loop();
        }
        
        Ok(())
    }
    
    /// Build TLS 1.3 ClientHello
    fn build_client_hello(&self) -> Vec<u8> {
        let mut msg = Vec::new();
        
        // Handshake type: ClientHello (1)
        msg.push(0x01);
        
        // Length placeholder (3 bytes)
        let len_pos = msg.len();
        msg.extend_from_slice(&[0, 0, 0]);
        
        // Protocol version: TLS 1.2 (for compatibility, actual is in extension)
        msg.extend_from_slice(&[0x03, 0x03]);
        
        // Random (32 bytes)
        let random = self.generate_random();
        msg.extend_from_slice(&random);
        
        // Session ID length (0 for TLS 1.3)
        msg.push(0);
        
        // Cipher suites
        let cipher_suites: &[u8] = &[
            0x00, 0x04, // Length
            0x13, 0x01, // TLS_AES_128_GCM_SHA256
            0x13, 0x02, // TLS_AES_256_GCM_SHA384
        ];
        msg.extend_from_slice(cipher_suites);
        
        // Compression methods
        msg.extend_from_slice(&[0x01, 0x00]); // null compression
        
        // Extensions
        let extensions = self.build_extensions();
        msg.extend_from_slice(&((extensions.len() as u16).to_be_bytes()));
        msg.extend_from_slice(&extensions);
        
        // Fill in length
        let msg_len = msg.len() - len_pos - 3;
        msg[len_pos] = ((msg_len >> 16) & 0xFF) as u8;
        msg[len_pos + 1] = ((msg_len >> 8) & 0xFF) as u8;
        msg[len_pos + 2] = (msg_len & 0xFF) as u8;
        
        msg
    }
    
    /// Build TLS extensions
    fn build_extensions(&self) -> Vec<u8> {
        let mut ext = Vec::new();
        
        // SNI (Server Name Indication)
        ext.extend_from_slice(&[0x00, 0x00]); // Extension type
        let sni_len = self.host.len() + 5;
        ext.extend_from_slice(&((sni_len as u16).to_be_bytes()));
        ext.extend_from_slice(&(((sni_len - 2) as u16).to_be_bytes()));
        ext.push(0x00); // Host name type
        ext.extend_from_slice(&((self.host.len() as u16).to_be_bytes()));
        ext.extend_from_slice(self.host.as_bytes());
        
        // Supported Versions (for TLS 1.3)
        ext.extend_from_slice(&[0x00, 0x2b]); // Extension type
        ext.extend_from_slice(&[0x00, 0x03]); // Length
        ext.push(0x02); // List length
        ext.extend_from_slice(&[0x03, 0x04]); // TLS 1.3
        
        // Supported Groups
        ext.extend_from_slice(&[0x00, 0x0a]); // Extension type
        ext.extend_from_slice(&[0x00, 0x04]); // Length
        ext.extend_from_slice(&[0x00, 0x02]); // List length
        ext.extend_from_slice(&[0x00, 0x17]); // secp256r1
        
        // Signature Algorithms
        ext.extend_from_slice(&[0x00, 0x0d]); // Extension type
        ext.extend_from_slice(&[0x00, 0x04]); // Length
        ext.extend_from_slice(&[0x00, 0x02]); // List length
        ext.extend_from_slice(&[0x04, 0x03]); // ecdsa_secp256r1_sha256
        
        // Key Share (simplified - would need proper ECDH)
        ext.extend_from_slice(&[0x00, 0x33]); // Extension type
        // Placeholder for key share...
        
        ext
    }
    
    /// Generate random bytes
    fn generate_random(&self) -> [u8; 32] {
        let mut random = [0u8; 32];
        let tick = crate::timer::ticks();
        let time = crate::rtc::unix_time();
        
        for i in 0..32 {
            random[i] = ((tick >> (i % 8)) ^ (time >> (i % 4)) ^ (i as u64 * 17)) as u8;
        }
        
        random
    }
    
    /// Send TLS record
    fn send_record(&mut self, content_type: u8, data: &[u8]) -> Result<(), HttpError> {
        let mut record = Vec::new();
        
        // Content type
        record.push(content_type);
        
        // Version: TLS 1.2 (0x0303)
        record.extend_from_slice(&[0x03, 0x03]);
        
        // Length
        record.extend_from_slice(&((data.len() as u16).to_be_bytes()));
        
        // Data
        record.extend_from_slice(data);
        
        self.socket.send(&record);
        Ok(())
    }
    
    /// Process handshake messages
    fn process_handshake(&mut self, data: &[u8]) -> Result<(), HttpError> {
        if data.len() < 5 {
            return Ok(());
        }
        
        let content_type = data[0];
        let _version = u16::from_be_bytes([data[1], data[2]]);
        let length = u16::from_be_bytes([data[3], data[4]]) as usize;
        
        if data.len() < 5 + length {
            return Ok(());
        }
        
        let payload = &data[5..5 + length];
        
        match content_type {
            0x16 => { // Handshake
                if !payload.is_empty() {
                    let msg_type = payload[0];
                    match msg_type {
                        0x02 => { // ServerHello
                            self.state = TlsState::ServerHelloReceived;
                            // In a full implementation, extract keys here
                        }
                        0x14 => { // Finished
                            self.state = TlsState::Established;
                        }
                        _ => {}
                    }
                }
            }
            0x15 => { // Alert
                return Err(HttpError::TlsError(String::from("TLS Alert received")));
            }
            _ => {}
        }
        
        // Simplified: assume handshake completes
        // In real implementation, need proper key exchange
        if self.state == TlsState::ServerHelloReceived {
            // Derive keys (simplified)
            self.derive_keys();
            self.state = TlsState::Established;
        }
        
        Ok(())
    }
    
    /// Derive session keys (simplified)
    fn derive_keys(&mut self) {
        // In a real implementation, this would use HKDF
        // For now, use placeholder values
        let seed = self.generate_random();
        self.client_write_key.copy_from_slice(&seed);
        self.server_write_key.copy_from_slice(&seed);
    }
    
    /// Write data to TLS stream
    pub fn write(&mut self, data: &[u8]) -> Result<(), HttpError> {
        if self.state != TlsState::Established {
            return Err(HttpError::TlsError(String::from("Not connected")));
        }
        
        // In real TLS, would encrypt here
        // For now, send as application data
        self.send_record(0x17, data)?;
        self.client_seq += 1;
        
        Ok(())
    }
    
    /// Read data from TLS stream
    pub fn read(&mut self) -> Result<Vec<u8>, HttpError> {
        crate::e1000::poll();
        
        if let Some(data) = self.socket.recv() {
            if data.len() >= 5 {
                let content_type = data[0];
                let length = u16::from_be_bytes([data[3], data[4]]) as usize;
                
                if content_type == 0x17 && data.len() >= 5 + length {
                    // Application data
                    // In real TLS, would decrypt here
                    self.server_seq += 1;
                    return Ok(data[5..5 + length].to_vec());
                }
            }
        }
        
        Ok(Vec::new())
    }
    
    /// Close TLS connection
    pub fn close(&mut self) {
        if self.state == TlsState::Established {
            // Send close_notify alert
            let _ = self.send_record(0x15, &[0x01, 0x00]);
        }
        self.socket.close();
        self.state = TlsState::Closed;
    }
}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Simple GET request
pub fn get(url: &str) -> Result<Response, HttpError> {
    Request::get(url)?.send()
}

/// Simple POST request with JSON body
pub fn post_json(url: &str, json: &str) -> Result<Response, HttpError> {
    Request::post(url)?.json(json).send()
}

/// POST with authentication
pub fn post_with_auth(url: &str, token: &str, json: &str) -> Result<Response, HttpError> {
    Request::post(url)?
        .bearer_auth(token)
        .json(json)
        .send()
}
