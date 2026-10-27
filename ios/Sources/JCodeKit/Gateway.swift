import Foundation

/// Describes a jcode server gateway endpoint.
///
/// The server exposes:
/// - `GET  http://host:port/health`  reachability + version probe
/// - `POST http://host:port/pair`    pairing-code -> auth-token exchange
/// - `ws://host:port/ws`             newline-delimited JSON protocol over WebSocket
public struct Gateway: Hashable, Sendable {
    public static let defaultPort: UInt16 = 7643

    /// How the gateway is reached. `https` also implies `wss` for the socket.
    ///
    /// macOS builds cannot open cleartext connections to a tailnet address:
    /// App Transport Security ignores `NSAllowsArbitraryLoads` off iOS and
    /// refuses `http://100.x` outright. Terminating TLS with
    /// `tailscale serve --https=443 http://127.0.0.1:7643` gives the client a
    /// publicly trusted certificate on the tailnet name, so the same server
    /// works from the Mac without a tunnel.
    public enum Security: String, Codable, Hashable, Sendable, CaseIterable {
        case http
        case https

        /// Port used when the address does not name one.
        public var defaultPort: UInt16 {
            switch self {
            case .http: Gateway.defaultPort
            case .https: 443
            }
        }

        public var webSocketScheme: String {
            switch self {
            case .http: "ws"
            case .https: "wss"
            }
        }
    }

    public var host: String
    public var port: UInt16
    public var security: Security

    public init(
        host: String,
        port: UInt16 = Gateway.defaultPort,
        security: Security = .http
    ) {
        self.host = host
        self.port = port
        self.security = security
    }

    public var healthURL: URL {
        url(scheme: security.rawValue, path: "/health")
    }

    public var pairURL: URL {
        url(scheme: security.rawValue, path: "/pair")
    }

    public var webSocketURL: URL {
        url(scheme: security.webSocketScheme, path: "/ws")
    }

    /// Address to show in the UI, omitting the scheme's default port.
    public var displayAddress: String {
        let base = "\(security.rawValue)://\(host)"
        return port == security.defaultPort ? base : "\(base):\(port)"
    }

    /// Parses what a person types into the host field: a bare host, a
    /// `host:port` pair, or a full `http(s)://` (or `ws(s)://`) URL.
    ///
    /// `port` is used only when the input does not carry one. Returns nil when
    /// the input cannot describe a gateway.
    public static func parse(_ input: String, port: UInt16 = Gateway.defaultPort) -> Gateway? {
        var text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }

        if text.contains("://") {
            guard let components = URLComponents(string: text),
                let scheme = components.scheme?.lowercased(),
                let security = Security(rawValue: scheme)
                    ?? Security(rawValue: scheme.replacingOccurrences(of: "ws", with: "http")),
                let host = components.host, !host.isEmpty
            else { return nil }
            return Gateway(
                host: host,
                port: components.port.flatMap(UInt16.init) ?? security.defaultPort,
                security: security
            )
        }

        while text.hasSuffix("/") { text.removeLast() }
        guard !text.isEmpty else { return nil }

        // `[v6]` / `[v6]:port`
        if text.hasPrefix("[") {
            guard let close = text.firstIndex(of: "]") else { return nil }
            let host = String(text[text.index(after: text.startIndex)..<close])
            let rest = text[text.index(after: close)...]
            guard !host.isEmpty else { return nil }
            if rest.isEmpty {
                return Gateway(host: host, port: port, security: port == 443 ? .https : .http)
            }
            guard rest.hasPrefix(":"), let parsed = UInt16(rest.dropFirst()) else { return nil }
            return Gateway(host: host, port: parsed)
        }

        // `host:port`. A colon that is not a numeric port suffix is a typo,
        // not part of a hostname — report it instead of dialing nonsense.
        if let colon = text.lastIndex(of: ":") {
            let head = String(text[text.startIndex..<colon])
            guard !head.isEmpty, !head.contains(":"), let parsed = UInt16(text[text.index(after: colon)...])
            else { return nil }
            return Gateway(host: head, port: parsed)
        }

        // A tailnet name paired with 443 is almost always the served name.
        let security: Security = port == 443 ? .https : .http
        return Gateway(host: text, port: port, security: security)
    }

    private func url(scheme: String, path: String) -> URL {
        var components = URLComponents()
        components.scheme = scheme
        components.host = host
        components.port = Int(port)
        components.path = path
        guard let url = components.url else {
            // Host strings from pairing are validated before reaching here;
            // a failure indicates programmer error.
            preconditionFailure("invalid gateway endpoint: \(scheme)://\(host):\(port)\(path)")
        }
        return url
    }
}

/// Parses `jcode://pair?host=H&port=P&code=C` URIs from QR codes.
public enum PairURI {
    public struct Payload: Equatable, Sendable {
        public var gateway: Gateway
        public var code: String

        public init(gateway: Gateway, code: String) {
            self.gateway = gateway
            self.code = code
        }
    }

    public static func parse(_ string: String) -> Payload? {
        guard let components = URLComponents(string: string),
            components.scheme == "jcode",
            components.host == "pair" || components.path == "pair"
                || components.host == nil && components.path == "/pair"
        else { return nil }
        let items = components.queryItems ?? []
        func value(_ name: String) -> String? {
            items.first(where: { $0.name == name })?.value
        }
        guard let host = value("host"), !host.isEmpty,
            let code = value("code"), !code.isEmpty
        else { return nil }
        let port = value("port").flatMap(UInt16.init)
        let security: Gateway.Security =
            value("scheme").flatMap(Gateway.Security.init(rawValue:))
            ?? (value("tls").map { $0 == "1" || $0.lowercased() == "true" } ?? false ? .https : nil)
            ?? ((port ?? Gateway.defaultPort) == 443 ? .https : .http)
        return Payload(
            gateway: Gateway(
                host: host,
                port: port ?? security.defaultPort,
                security: security
            ),
            code: code
        )
    }
}
