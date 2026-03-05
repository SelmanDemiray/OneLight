///! Homemade HTTP client — uses system TLS (WinHTTP on Windows, raw sockets on Linux).
///! No third-party crates. Handles HTTP/1.1 GET requests, chunked transfer encoding,
///! redirects, and bearer token auth for Docker Registry protocol.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;

use crate::error::{ContainerError, Result};

// ─── HTTP Types ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn body_string(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(&key.to_lowercase()).map(|s| s.as_str())
    }
}

// ─── URL Parser ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Url {
    pub fn parse(url: &str) -> Result<Url> {
        let (scheme, rest) = if url.starts_with("https://") {
            ("https".to_string(), &url[8..])
        } else if url.starts_with("http://") {
            ("http".to_string(), &url[7..])
        } else {
            return Err(ContainerError::Network(format!("unsupported URL scheme: {}", url)));
        };

        let (host_port, path) = match rest.find('/') {
            Some(i) => (&rest[..i], rest[i..].to_string()),
            None => (rest, "/".to_string()),
        };

        let (host, port) = match host_port.find(':') {
            Some(i) => {
                let p: u16 = host_port[i+1..].parse()
                    .map_err(|_| ContainerError::Network("invalid port".into()))?;
                (host_port[..i].to_string(), p)
            }
            None => {
                let p = if scheme == "https" { 443 } else { 80 };
                (host_port.to_string(), p)
            }
        };

        Ok(Url { scheme, host, port, path })
    }

    pub fn host_header(&self) -> String {
        if (self.scheme == "https" && self.port == 443) ||
           (self.scheme == "http" && self.port == 80) {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

// ─── Platform-specific TLS ──────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod tls {
    use super::*;

    // WinHTTP FFI bindings — hand-declared, no winapi crate
    #[allow(non_snake_case, non_camel_case_types)]
    mod winhttp {
        pub type HINTERNET = *mut std::ffi::c_void;
        pub type DWORD = u32;
        pub type LPCWSTR = *const u16;
        pub type LPVOID = *mut std::ffi::c_void;
        pub type BOOL = i32;
        pub type WORD = u16;

        pub const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: DWORD = 4;
        pub const WINHTTP_FLAG_SECURE: DWORD = 0x00800000;
        pub const WINHTTP_ADDREQ_FLAG_ADD: DWORD = 0x20000000;
        pub const WINHTTP_QUERY_STATUS_CODE: DWORD = 19;
        pub const WINHTTP_QUERY_FLAG_NUMBER: DWORD = 0x20000000;
        pub const WINHTTP_QUERY_RAW_HEADERS_CRLF: DWORD = 22;
        pub const WINHTTP_NO_REFERER: LPCWSTR = std::ptr::null();
        pub const WINHTTP_DEFAULT_ACCEPT_TYPES: *const LPCWSTR = std::ptr::null();
        pub const WINHTTP_NO_ADDITIONAL_HEADERS: LPCWSTR = std::ptr::null();
        pub const WINHTTP_NO_REQUEST_DATA: LPVOID = std::ptr::null_mut();
        pub const WINHTTP_OPTION_SECURITY_FLAGS: DWORD = 31;
        pub const SECURITY_FLAG_IGNORE_UNKNOWN_CA: DWORD = 0x00000100;
        pub const SECURITY_FLAG_IGNORE_CERT_DATE_INVALID: DWORD = 0x00002000;
        pub const SECURITY_FLAG_IGNORE_CERT_CN_INVALID: DWORD = 0x00001000;
        pub const SECURITY_FLAG_IGNORE_CERT_WRONG_USAGE: DWORD = 0x00000200;
        pub const INTERNET_DEFAULT_HTTPS_PORT: WORD = 443;
        pub const INTERNET_DEFAULT_HTTP_PORT: WORD = 80;
        pub const WINHTTP_OPTION_REDIRECT_POLICY: DWORD = 88;
        pub const WINHTTP_OPTION_REDIRECT_POLICY_NEVER: DWORD = 0;

        #[link(name = "winhttp")]
        extern "system" {
            pub fn WinHttpOpen(
                pszAgentW: LPCWSTR,
                dwAccessType: DWORD,
                pszProxyW: LPCWSTR,
                pszProxyBypassW: LPCWSTR,
                dwFlags: DWORD,
            ) -> HINTERNET;

            pub fn WinHttpConnect(
                hSession: HINTERNET,
                pswzServerName: LPCWSTR,
                nServerPort: WORD,
                dwReserved: DWORD,
            ) -> HINTERNET;

            pub fn WinHttpOpenRequest(
                hConnect: HINTERNET,
                pwszVerb: LPCWSTR,
                pwszObjectName: LPCWSTR,
                pwszVersion: LPCWSTR,
                pwszReferrer: LPCWSTR,
                ppwszAcceptTypes: *const LPCWSTR,
                dwFlags: DWORD,
            ) -> HINTERNET;

            pub fn WinHttpAddRequestHeaders(
                hRequest: HINTERNET,
                lpszHeaders: LPCWSTR,
                dwHeadersLength: DWORD,
                dwModifiers: DWORD,
            ) -> BOOL;

            pub fn WinHttpSendRequest(
                hRequest: HINTERNET,
                lpszHeaders: LPCWSTR,
                dwHeadersLength: DWORD,
                lpOptional: LPVOID,
                dwOptionalLength: DWORD,
                dwTotalLength: DWORD,
                dwContext: usize,
            ) -> BOOL;

            pub fn WinHttpReceiveResponse(
                hRequest: HINTERNET,
                lpReserved: LPVOID,
            ) -> BOOL;

            pub fn WinHttpQueryHeaders(
                hRequest: HINTERNET,
                dwInfoLevel: DWORD,
                pwszName: LPCWSTR,
                lpBuffer: LPVOID,
                lpdwBufferLength: *mut DWORD,
                lpdwIndex: *mut DWORD,
            ) -> BOOL;

            pub fn WinHttpReadData(
                hRequest: HINTERNET,
                lpBuffer: LPVOID,
                dwNumberOfBytesToRead: DWORD,
                lpdwNumberOfBytesRead: *mut DWORD,
            ) -> BOOL;

            pub fn WinHttpCloseHandle(hInternet: HINTERNET) -> BOOL;

            pub fn WinHttpSetOption(
                hInternet: HINTERNET,
                dwOption: DWORD,
                lpBuffer: LPVOID,
                dwBufferLength: DWORD,
            ) -> BOOL;

            pub fn WinHttpQueryDataAvailable(
                hRequest: HINTERNET,
                lpdwNumberOfBytesAvailable: *mut DWORD,
            ) -> BOOL;
        }
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn https_get(url: &Url, headers: &[(&str, &str)]) -> Result<HttpResponse> {
        unsafe {
            let agent = to_wide("HolyContainer/1.0");
            let session = winhttp::WinHttpOpen(
                agent.as_ptr(),
                winhttp::WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            );
            if session.is_null() {
                return Err(ContainerError::Network("WinHttpOpen failed".into()));
            }

            let host_w = to_wide(&url.host);
            let port = url.port as winhttp::WORD;
            let connect = winhttp::WinHttpConnect(session, host_w.as_ptr(), port, 0);
            if connect.is_null() {
                winhttp::WinHttpCloseHandle(session);
                return Err(ContainerError::Network("WinHttpConnect failed".into()));
            }

            let verb = to_wide("GET");
            let path_w = to_wide(&url.path);
            let flags = if url.scheme == "https" { winhttp::WINHTTP_FLAG_SECURE } else { 0 };

            let request = winhttp::WinHttpOpenRequest(
                connect,
                verb.as_ptr(),
                path_w.as_ptr(),
                std::ptr::null(),
                winhttp::WINHTTP_NO_REFERER,
                winhttp::WINHTTP_DEFAULT_ACCEPT_TYPES,
                flags,
            );
            if request.is_null() {
                winhttp::WinHttpCloseHandle(connect);
                winhttp::WinHttpCloseHandle(session);
                return Err(ContainerError::Network("WinHttpOpenRequest failed".into()));
            }

            // Disable auto-redirect so we can strip auth headers on cross-origin redirects
            let mut redirect_policy = winhttp::WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
            winhttp::WinHttpSetOption(
                request,
                winhttp::WINHTTP_OPTION_REDIRECT_POLICY,
                &mut redirect_policy as *mut u32 as winhttp::LPVOID,
                std::mem::size_of::<u32>() as u32,
            );

            // Add custom headers
            for (key, value) in headers {
                let header = to_wide(&format!("{}: {}", key, value));
                winhttp::WinHttpAddRequestHeaders(
                    request,
                    header.as_ptr(),
                    header.len() as u32 - 1, // exclude null
                    winhttp::WINHTTP_ADDREQ_FLAG_ADD,
                );
            }

            // Send request
            if winhttp::WinHttpSendRequest(
                request,
                winhttp::WINHTTP_NO_ADDITIONAL_HEADERS,
                0,
                winhttp::WINHTTP_NO_REQUEST_DATA,
                0, 0, 0,
            ) == 0 {
                winhttp::WinHttpCloseHandle(request);
                winhttp::WinHttpCloseHandle(connect);
                winhttp::WinHttpCloseHandle(session);
                return Err(ContainerError::Network("WinHttpSendRequest failed".into()));
            }

            // Receive response
            if winhttp::WinHttpReceiveResponse(request, std::ptr::null_mut()) == 0 {
                winhttp::WinHttpCloseHandle(request);
                winhttp::WinHttpCloseHandle(connect);
                winhttp::WinHttpCloseHandle(session);
                return Err(ContainerError::Network("WinHttpReceiveResponse failed".into()));
            }

            // Get status code
            let mut status_code: u32 = 0;
            let mut size: u32 = std::mem::size_of::<u32>() as u32;
            winhttp::WinHttpQueryHeaders(
                request,
                winhttp::WINHTTP_QUERY_STATUS_CODE | winhttp::WINHTTP_QUERY_FLAG_NUMBER,
                std::ptr::null(),
                &mut status_code as *mut u32 as winhttp::LPVOID,
                &mut size,
                std::ptr::null_mut(),
            );

            // Get response headers
            let mut header_size: u32 = 0;
            winhttp::WinHttpQueryHeaders(
                request,
                winhttp::WINHTTP_QUERY_RAW_HEADERS_CRLF,
                std::ptr::null(),
                std::ptr::null_mut(),
                &mut header_size,
                std::ptr::null_mut(),
            );

            let mut resp_headers = HashMap::new();
            if header_size > 0 {
                let mut header_buf: Vec<u16> = vec![0u16; (header_size / 2) as usize + 1];
                winhttp::WinHttpQueryHeaders(
                    request,
                    winhttp::WINHTTP_QUERY_RAW_HEADERS_CRLF,
                    std::ptr::null(),
                    header_buf.as_mut_ptr() as winhttp::LPVOID,
                    &mut header_size,
                    std::ptr::null_mut(),
                );
                let header_str = String::from_utf16_lossy(&header_buf);
                for line in header_str.lines() {
                    if let Some(colon) = line.find(':') {
                        let key = line[..colon].trim().to_lowercase();
                        let value = line[colon+1..].trim().to_string();
                        resp_headers.insert(key, value);
                    }
                }
            }

            // Read body
            let mut body = Vec::new();
            loop {
                let mut bytes_available: u32 = 0;
                if winhttp::WinHttpQueryDataAvailable(request, &mut bytes_available) == 0 {
                    break;
                }
                if bytes_available == 0 {
                    break;
                }
                let mut buf = vec![0u8; bytes_available as usize];
                let mut bytes_read: u32 = 0;
                if winhttp::WinHttpReadData(
                    request,
                    buf.as_mut_ptr() as winhttp::LPVOID,
                    bytes_available,
                    &mut bytes_read,
                ) == 0 {
                    break;
                }
                body.extend_from_slice(&buf[..bytes_read as usize]);
            }

            winhttp::WinHttpCloseHandle(request);
            winhttp::WinHttpCloseHandle(connect);
            winhttp::WinHttpCloseHandle(session);

            Ok(HttpResponse {
                status: status_code as u16,
                headers: resp_headers,
                body,
            })
        }
    }
}

#[cfg(target_os = "linux")]
mod tls {
    use super::*;

    /// On Linux, use raw TCP for HTTP, or dynamically load libssl for HTTPS.
    /// For simplicity, we first try plain HTTP. If HTTPS is needed,
    /// we use a subprocess call to `curl` as a fallback (still no Rust deps).
    /// A future version can implement raw TLS via dlopen("libssl.so").
    pub fn https_get(url: &Url, headers: &[(&str, &str)]) -> Result<HttpResponse> {
        if url.scheme == "http" {
            return http_get_raw(url, headers);
        }

        // For HTTPS on Linux, use raw socket + system TLS via subprocess
        // This is a pragmatic bridge — the pure-Rust TLS can be added later
        https_get_via_subprocess(url, headers)
    }

    fn http_get_raw(url: &Url, headers: &[(&str, &str)]) -> Result<HttpResponse> {
        let addr = format!("{}:{}", url.host, url.port);
        let mut stream = TcpStream::connect(&addr)
            .map_err(|e| ContainerError::Network(format!("connect to {}: {}", addr, e)))?;

        // Build request
        let mut request = format!("GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            url.path, url.host_header());

        for (key, value) in headers {
            request.push_str(&format!("{}: {}\r\n", key, value));
        }
        request.push_str("\r\n");

        stream.write_all(request.as_bytes())
            .map_err(|e| ContainerError::Network(format!("write: {}", e)))?;

        let mut response_bytes = Vec::new();
        stream.read_to_end(&mut response_bytes)
            .map_err(|e| ContainerError::Network(format!("read: {}", e)))?;

        parse_http_response(&response_bytes)
    }

    fn https_get_via_subprocess(url: &Url, headers: &[(&str, &str)]) -> Result<HttpResponse> {
        use std::process::Command;

        let full_url = format!("{}://{}:{}{}", url.scheme, url.host, url.port, url.path);

        let mut cmd = Command::new("curl");
        cmd.arg("-s").arg("-i").arg("-L");

        for (key, value) in headers {
            cmd.arg("-H").arg(format!("{}: {}", key, value));
        }

        cmd.arg(&full_url);

        let output = cmd.output()
            .map_err(|e| ContainerError::Network(format!("curl: {}", e)))?;

        if !output.status.success() && output.stdout.is_empty() {
            return Err(ContainerError::Network(format!(
                "curl failed: {}", String::from_utf8_lossy(&output.stderr)
            )));
        }

        parse_http_response(&output.stdout)
    }
}

// ─── HTTP Response Parser ───────────────────────────────────────────────────

fn parse_http_response(data: &[u8]) -> Result<HttpResponse> {
    // Find end of headers
    let header_end = find_header_end(data)
        .ok_or_else(|| ContainerError::Network("malformed HTTP response".into()))?;

    let header_bytes = &data[..header_end];
    let body = data[header_end + 4..].to_vec(); // skip \r\n\r\n

    let header_str = String::from_utf8_lossy(header_bytes);
    let mut lines = header_str.lines();

    // Parse status line
    let status_line = lines.next()
        .ok_or_else(|| ContainerError::Network("missing status line".into()))?;

    let status: u16 = status_line.split_whitespace().nth(1)
        .ok_or_else(|| ContainerError::Network("missing status code".into()))?
        .parse()
        .map_err(|_| ContainerError::Network("invalid status code".into()))?;

    // Parse headers
    let mut headers = HashMap::new();
    for line in lines {
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_lowercase();
            let value = line[colon+1..].trim().to_string();
            headers.insert(key, value);
        }
    }

    // Handle chunked transfer encoding
    let final_body = if headers.get("transfer-encoding").map(|v| v.contains("chunked")).unwrap_or(false) {
        decode_chunked(&body)?
    } else {
        body
    };

    Ok(HttpResponse { status, headers, body: final_body })
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(3) {
        if &data[i..i+4] == b"\r\n\r\n" {
            return Some(i);
        }
    }
    None
}

fn decode_chunked(data: &[u8]) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut pos = 0;

    loop {
        // Find chunk size line
        let line_end = match find_crlf(&data[pos..]) {
            Some(i) => i,
            None => break,
        };

        let size_str = std::str::from_utf8(&data[pos..pos + line_end])
            .map_err(|_| ContainerError::Network("invalid chunk size".into()))?;

        let chunk_size = usize::from_str_radix(size_str.trim(), 16)
            .map_err(|_| ContainerError::Network(format!("invalid chunk size: '{}'", size_str)))?;

        if chunk_size == 0 {
            break;
        }

        pos += line_end + 2; // skip size line + \r\n

        if pos + chunk_size > data.len() {
            break;
        }

        result.extend_from_slice(&data[pos..pos + chunk_size]);
        pos += chunk_size + 2; // skip chunk data + \r\n
    }

    Ok(result)
}

fn find_crlf(data: &[u8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(1) {
        if data[i] == b'\r' && data[i + 1] == b'\n' {
            return Some(i);
        }
    }
    None
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Perform an HTTP GET request with optional headers.
pub fn get(url_str: &str, headers: &[(&str, &str)]) -> Result<HttpResponse> {
    let url = Url::parse(url_str)?;
    tls::https_get(&url, headers)
}

/// Perform an HTTP GET request and follow redirects.
/// Strips Authorization headers on cross-origin redirects (required for Docker Hub CDN).
pub fn get_follow_redirects(url_str: &str, headers: &[(&str, &str)], max_redirects: u32) -> Result<HttpResponse> {
    let mut current_url = url_str.to_string();
    let original_host = Url::parse(url_str)?.host.clone();

    for _ in 0..max_redirects {
        // Determine if we're on the same origin
        let current_parsed = Url::parse(&current_url)?;
        let same_origin = current_parsed.host == original_host;

        // Only include auth headers if same origin
        let filtered_headers: Vec<(&str, &str)> = if same_origin {
            headers.to_vec()
        } else {
            headers.iter()
                .filter(|(k, _)| !k.eq_ignore_ascii_case("authorization"))
                .cloned()
                .collect()
        };

        let resp = get(&current_url, &filtered_headers)?;
        match resp.status {
            301 | 302 | 307 | 308 => {
                if let Some(location) = resp.header("location") {
                    current_url = if location.starts_with("http") {
                        location.to_string()
                    } else {
                        let url = Url::parse(&current_url)?;
                        format!("{}://{}:{}{}", url.scheme, url.host, url.port, location)
                    };
                    continue;
                }
                return Ok(resp);
            }
            _ => return Ok(resp),
        }
    }
    Err(ContainerError::Network("too many redirects".into()))
}

/// Download a URL to a file, with progress output.
pub fn download_to_file(url_str: &str, headers: &[(&str, &str)], path: &std::path::Path) -> Result<u64> {
    let resp = get_follow_redirects(url_str, headers, 5)?;

    if resp.status != 200 {
        return Err(ContainerError::Network(format!(
            "download failed: HTTP {}", resp.status
        )));
    }

    std::fs::write(path, &resp.body)
        .map_err(|e| ContainerError::Filesystem(format!("write {}: {}", path.display(), e)))?;

    Ok(resp.body.len() as u64)
}
