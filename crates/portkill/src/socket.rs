/// A process bound to a listening network socket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProcessSocket {
    /// Process ID.
    pub pid: u32,
    /// Command name (e.g. `node`, `bun`, `cargo`).
    pub command: String,
    /// Owning system username.
    pub user: String,
    /// Protocol (e.g. `TCP`, `UDP`).
    pub proto: String,
    /// Bound network port number.
    pub port: u16,
    /// Full address string (e.g. `127.0.0.1:3000`, `*:8080`).
    pub address: String,
    /// Socket state (e.g. `LISTEN`).
    pub state: String,
}

/// Extract port number from address string like `*:3000`, `127.0.0.1:8080`, `[::1]:5173`.
pub fn extract_port(address: &str) -> Option<u16> {
    let last_colon = address.rfind(':')?;
    let port_str = &address[last_colon + 1..];
    port_str.parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_port_handles_various_formats() {
        assert_eq!(extract_port("*:3000"), Some(3000));
        assert_eq!(extract_port("127.0.0.1:8080"), Some(8080));
        assert_eq!(extract_port("0.0.0.0:5173"), Some(5173));
        assert_eq!(extract_port("[::1]:9000"), Some(9000));
        assert_eq!(extract_port("invalid"), None);
    }
}
