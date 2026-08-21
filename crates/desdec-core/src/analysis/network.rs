//! Whether a file has what it takes to reach the network, and what says so.
//!
//! This is a statement about the file, never about a run: Desdec does not
//! execute what it opens, so nothing here means "this program contacted a
//! server". It means the code to do so is in the file — the names it asks a
//! system or a library for, and the libraries themselves — which is what a
//! reader wants to know before deciding how carefully to read the rest.
//!
//! Names are matched exactly, or by a prefix chosen to be unambiguous. That
//! matters more than it sounds: a file that calls `g_signal_connect` connects
//! a button to a callback and touches no socket at all, and a reader shown a
//! red flag for it would be right never to trust the next one. Where the whole
//! network stack is compiled into the file — a Go or Rust binary that imports
//! nothing — the evidence is instead the name of a function *inside* it, so a
//! few such fragments are looked for in defined symbols too.

use super::{Symbol, details::BinaryDetails};

/// What a name says the file can do.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Reach {
    /// Opens a connection, or takes one that is offered.
    Connect,
    /// Sends bytes out.
    Send,
    /// Takes bytes in.
    Receive,
    /// Turns a name into an address, which is itself a question asked of a
    /// server somewhere.
    Resolve,
    /// Speaks something above the socket — HTTP, TLS — which does both.
    Protocol,
}

/// One name found in the file, and what it says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkName {
    pub name: String,
    pub reach: Reach,
}

/// Everything in a file that says it can reach the network.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NetworkUse {
    /// The names, sorted and without repeats, so the same list comes out of
    /// the same file whatever order its tables were in.
    pub names: Vec<NetworkName>,
    /// Libraries whose whole business is the network. Named as the file spells
    /// them, minus the path a Mach-O framework carries.
    pub libraries: Vec<String>,
}

impl NetworkUse {
    /// Whether the file says nothing about the network at all.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.names.is_empty() && self.libraries.is_empty()
    }

    /// Whether anything found puts bytes on the wire.
    ///
    /// A library on its own does not answer this: linking `libcurl` says the
    /// road is there, not that anything drives down it. A name does.
    #[must_use]
    pub fn sends(&self) -> bool {
        self.does(&[Reach::Send, Reach::Protocol, Reach::Resolve])
    }

    /// Whether anything found takes bytes off it.
    #[must_use]
    pub fn receives(&self) -> bool {
        self.does(&[Reach::Receive, Reach::Protocol, Reach::Resolve])
    }

    fn does(&self, reaches: &[Reach]) -> bool {
        self.names.iter().any(|name| reaches.contains(&name.reach))
    }
}

/// Names that mean the network wherever they appear, with what each does.
///
/// Exact matches only. The list is of the interfaces a program actually calls
/// — the BSD sockets every system carries, and the Windows spellings of the
/// same — rather than of everything that has ever touched a packet.
const EXACT: &[(&str, Reach)] = &[
    // BSD sockets, which Linux, macOS and Windows all speak.
    ("socket", Reach::Connect),
    ("socketpair", Reach::Connect),
    ("connect", Reach::Connect),
    ("bind", Reach::Connect),
    ("listen", Reach::Connect),
    ("accept", Reach::Connect),
    ("accept4", Reach::Connect),
    ("closesocket", Reach::Connect),
    ("send", Reach::Send),
    ("sendto", Reach::Send),
    ("sendmsg", Reach::Send),
    ("sendmmsg", Reach::Send),
    ("sendfile", Reach::Send),
    ("recv", Reach::Receive),
    ("recvfrom", Reach::Receive),
    ("recvmsg", Reach::Receive),
    ("recvmmsg", Reach::Receive),
    // Names into addresses: a question for a resolver, which is a server.
    ("getaddrinfo", Reach::Resolve),
    ("getnameinfo", Reach::Resolve),
    ("gethostbyname", Reach::Resolve),
    ("gethostbyname2", Reach::Resolve),
    ("gethostbyaddr", Reach::Resolve),
    ("res_query", Reach::Resolve),
    ("res_search", Reach::Resolve),
    ("DnsQuery_A", Reach::Resolve),
    ("DnsQuery_W", Reach::Resolve),
    // Above the socket.
    ("URLDownloadToFileA", Reach::Protocol),
    ("URLDownloadToFileW", Reach::Protocol),
    ("SSLHandshake", Reach::Protocol),
    ("SSLRead", Reach::Receive),
    ("SSLWrite", Reach::Send),
];

/// Prefixes that are only ever the network, with what the family does.
///
/// A prefix is used where a family shares one, so that a spelling this list
/// has never heard of — a new `WinHttp` call, a wide-character twin — is still
/// recognised. Each is narrow enough that nothing else in a binary starts with
/// it.
const PREFIXES: &[(&str, Reach)] = &[
    ("WSA", Reach::Connect),
    ("SSL_", Reach::Protocol),
    ("TLS_", Reach::Protocol),
    ("curl_easy_", Reach::Protocol),
    ("curl_multi_", Reach::Protocol),
    ("curl_global_", Reach::Protocol),
    ("InternetOpen", Reach::Protocol),
    ("InternetConnect", Reach::Connect),
    ("InternetRead", Reach::Receive),
    ("InternetWrite", Reach::Send),
    ("HttpOpenRequest", Reach::Protocol),
    ("HttpSendRequest", Reach::Send),
    ("HttpQueryInfo", Reach::Receive),
    ("WinHttp", Reach::Protocol),
    ("nw_connection_", Reach::Connect),
    ("nw_endpoint_", Reach::Connect),
    ("CFNetwork", Reach::Protocol),
    ("CFReadStreamCreateForHTTPRequest", Reach::Protocol),
    ("NSURLSession", Reach::Protocol),
    ("NSURLConnection", Reach::Protocol),
];

/// Fragments looked for inside any symbol, for files that import nothing.
///
/// A Go or Rust binary carries its whole network stack, so the evidence is the
/// name of a function it was built from. Each fragment names a package path or
/// a type whose only purpose is the network; a bare word like `connect` is
/// deliberately not here, since it would match half of a user interface.
const FRAGMENTS: &[(&str, Reach)] = &[
    // Rust as the linker spells it, since nothing here demangles: `3std3net`
    // is what `std::net` becomes, and the length-prefixed form is far too
    // particular to appear by accident.
    //
    // Named down to the module that does the work, not up at `net`: every
    // Rust binary ever built carries `core::net`, which is the *parser* for
    // `IpAddr` and touches nothing. Flagging that would flag everything, and
    // a flag on everything says nothing.
    ("3std3net3tcp", Reach::Connect),
    ("3std3net3udp", Reach::Connect),
    ("3std3net11lookup_host", Reach::Resolve),
    ("5tokio3net", Reach::Connect),
    ("6rustls", Reach::Protocol),
    ("7reqwest", Reach::Protocol),
    ("5hyper6client", Reach::Protocol),
    // And demangled, for a listing that arrives already readable.
    ("std::net::TcpStream", Reach::Connect),
    ("std::net::UdpSocket", Reach::Connect),
    ("std::net::TcpListener", Reach::Connect),
    ("tokio::net::", Reach::Connect),
    ("hyper::client", Reach::Protocol),
    ("hyper::server", Reach::Protocol),
    ("reqwest::", Reach::Protocol),
    ("rustls::", Reach::Protocol),
    ("native_tls::", Reach::Protocol),
    ("net/http.", Reach::Protocol),
    ("crypto/tls.", Reach::Protocol),
    ("net.Dial", Reach::Connect),
    ("net.Listen", Reach::Connect),
    ("QNetworkAccessManager", Reach::Protocol),
    ("QTcpSocket", Reach::Connect),
    ("QUdpSocket", Reach::Connect),
    ("boost::asio::ip::", Reach::Connect),
];

/// Libraries that are the network and nothing else.
///
/// Matched on the file name alone, without its version or its path: a Mach-O
/// names a framework by a long path, and an ELF names `libcurl.so.4`.
const LIBRARIES: &[&str] = &[
    "ws2_32",
    "wsock32",
    "wininet",
    "winhttp",
    "dnsapi",
    "iphlpapi",
    "libcurl",
    "libssl",
    "libnsl",
    "libresolv",
    "libcares",
    "libsoup",
    "libnghttp2",
    // Windows file and printer sharing. Weaker evidence than the rest: the
    // same calls answer about the machine they run on when handed no server
    // name. It is listed all the same — a program that can read `\\server\share`
    // is a program that can reach another machine — and it lands as "can open
    // a connection" rather than as sending and receiving, since no name in it
    // says which.
    "netapi32",
    "cfnetwork",
    "network",
    "qtnetwork",
    "qt5network",
    "qt6network",
];

/// Reads what a file says about reaching the network.
#[must_use]
pub fn extract(symbols: &[Symbol], details: &BinaryDetails) -> NetworkUse {
    let mut names: Vec<NetworkName> = Vec::new();
    for symbol in symbols {
        if let Some(reach) = reach_of(&symbol.name) {
            names.push(NetworkName {
                name: symbol.name.clone(),
                reach,
            });
        }
    }
    // A PE names what it takes from each library, and those names are not in
    // the symbol table read above: without this, the one format that states
    // its imports plainly would be the one answering nothing.
    for imported in &details.imports {
        for function in &imported.functions {
            if let Some(reach) = reach_of(function) {
                names.push(NetworkName {
                    name: function.clone(),
                    reach,
                });
            }
        }
    }
    names.sort_by(|a, b| a.name.cmp(&b.name));
    names.dedup_by(|a, b| a.name == b.name);

    let mut libraries: Vec<String> = details
        .linked_libraries
        .iter()
        .filter(|library| is_network_library(library))
        .cloned()
        .collect();
    libraries.sort();
    libraries.dedup();

    NetworkUse { names, libraries }
}

/// What one name says, if anything.
fn reach_of(name: &str) -> Option<Reach> {
    // A Mach-O writes an underscore before every C name; the analysis strips
    // it where it reads symbols, but a name may still arrive with one.
    let bare = name.strip_prefix('_').unwrap_or(name);
    if let Some((_, reach)) = EXACT.iter().find(|(known, _)| *known == bare) {
        return Some(*reach);
    }
    if let Some((_, reach)) = PREFIXES
        .iter()
        .find(|(prefix, _)| bare.starts_with(prefix) && bare.len() > prefix.len())
    {
        return Some(*reach);
    }
    FRAGMENTS
        .iter()
        .find(|(fragment, _)| name.contains(fragment))
        .map(|(_, reach)| *reach)
}

/// Whether a library named by the file is a network library.
fn is_network_library(library: &str) -> bool {
    // The last component of a path, then the name before its version: a
    // framework arrives as `/System/…/CFNetwork.framework/…/CFNetwork`, and a
    // shared object as `libcurl.so.4`.
    let file = library
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(library)
        .to_ascii_lowercase();
    let stem = file
        .split_once(".so")
        .map_or(file.as_str(), |(stem, _)| stem)
        .trim_end_matches(".dll")
        .trim_end_matches(".dylib");
    LIBRARIES.contains(&stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::ImportedLibrary;

    fn symbol(name: &str) -> Symbol {
        Symbol {
            name: String::from(name),
            address: None,
            size: 0,
            imported: true,
        }
    }

    fn details(libraries: &[&str]) -> BinaryDetails {
        BinaryDetails {
            linked_libraries: libraries.iter().map(|name| String::from(*name)).collect(),
            ..BinaryDetails::default()
        }
    }

    #[test]
    fn sockets_say_the_file_sends_and_receives() {
        let symbols = [symbol("socket"), symbol("connect"), symbol("send")];
        let found = extract(&symbols, &details(&[]));
        assert!(!found.is_silent());
        assert!(found.sends());
        assert!(!found.receives(), "nothing here reads a byte back");

        let symbols = [symbol("recvfrom")];
        assert!(extract(&symbols, &details(&[])).receives());
    }

    /// The one mistake that would make the flag worthless: a user interface
    /// connecting a button to a callback is not a program on the network.
    #[test]
    fn a_name_that_merely_contains_a_network_word_is_not_evidence() {
        let symbols = [
            symbol("g_signal_connect"),
            symbol("g_signal_connect_data"),
            symbol("dbus_connection_send"),
            symbol("QObject::connect"),
            symbol("sqlite3_bind_text"),
        ];
        assert!(
            extract(&symbols, &details(&[])).is_silent(),
            "none of these is a socket"
        );
    }

    #[test]
    fn a_windows_import_table_is_read_as_well_as_the_symbols() {
        let mut binary = details(&["ws2_32.dll", "kernel32.dll"]);
        binary.imports = vec![ImportedLibrary {
            library: String::from("ws2_32.dll"),
            functions: vec![String::from("WSASend"), String::from("GetLastError")],
            truncated: false,
        }];
        let found = extract(&[], &binary);
        assert_eq!(found.libraries, vec![String::from("ws2_32.dll")]);
        assert_eq!(
            found.names,
            vec![NetworkName {
                name: String::from("WSASend"),
                reach: Reach::Connect,
            }]
        );
    }

    #[test]
    fn a_library_is_recognised_through_its_version_and_its_path() {
        assert!(is_network_library("libcurl.so.4"));
        assert!(is_network_library("libssl.so.3"));
        assert!(is_network_library(
            "/System/Library/Frameworks/CFNetwork.framework/Versions/A/CFNetwork"
        ));
        assert!(is_network_library("WS2_32.dll"));
        assert!(is_network_library("netapi32.dll"), "file sharing counts");
        assert!(!is_network_library("libc.so.6"));
        assert!(!is_network_library("libgtk-3.so.0"));
    }

    /// A file that carries its own stack imports nothing, so the evidence has
    /// to come from the names inside it.
    /// The fragment list is the one that could quietly ruin the flag: a name
    /// every binary of a language carries would light it up for all of them.
    #[test]
    fn a_type_that_only_describes_an_address_is_not_evidence() {
        let symbols = [
            // `core::net` is the parser for `IpAddr`, in every Rust binary
            // ever built, network or not.
            symbol("_RNvNtNtCs4NRVxsYgnAr_4core3net6parser11read_number"),
            symbol("_RNvNtNtCs4NRVxsYgnAr_4core3net11socket_addr10SocketAddr"),
            symbol("core::net::SocketAddr::new"),
        ];
        assert!(extract(&symbols, &details(&[])).is_silent());
    }

    #[test]
    fn a_statically_linked_stack_is_found_by_the_names_it_was_built_from() {
        // Mangled, which is how a Rust binary really spells it: nothing here
        // demangles, so the table has to hold the spelling the linker wrote.
        let symbols = [symbol(
            "_ZN3std3net3tcp9TcpStream7connect17h0e9f2b3c4d5e6f78E",
        )];
        assert!(!extract(&symbols, &details(&[])).is_silent());
        let symbols = [symbol("_RNvMNtNtCse_3std3net3tcp9TcpStream7connect")];
        assert!(!extract(&symbols, &details(&[])).is_silent());
        let symbols = [symbol("std::net::TcpStream::connect")];
        assert!(!extract(&symbols, &details(&[])).is_silent());
        let symbols = [symbol("net/http.(*Client).Do")];
        let found = extract(&symbols, &details(&[]));
        assert!(found.sends() && found.receives());
    }
}
