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
