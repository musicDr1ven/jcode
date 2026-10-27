import Foundation
import Testing

@testable import JCodeKit

// MARK: - Credentials saved by earlier builds

/// Pairings written before `security` and `workingDir` existed must keep
/// working: dropping them would force every tester to re-pair after upgrade.
@Test func decodesLegacyCredentialWithoutNewFields() throws {
    let legacy = """
        [{
          "host": "100.105.25.121",
          "port": 7643,
          "token": "tok",
          "serverName": "jcode",
          "serverVersion": "v0.99.1",
          "pairedAt": 774000000
        }]
        """
    let decoded = try JSONDecoder().decode(
        [ServerCredential].self,
        from: Data(legacy.utf8)
    )
    let credential = try #require(decoded.first)
    #expect(credential.host == "100.105.25.121")
    #expect(credential.security == .http)
    #expect(credential.workingDir.isEmpty)
    #expect(credential.gateway.webSocketURL.scheme == "ws")
}

@Test func roundTripsTLSCredential() throws {
    let credential = ServerCredential(
        host: "box.tailnet.ts.net",
        port: 443,
        security: .https,
        workingDir: "/home/gabe/project",
        token: "tok",
        serverName: "jcode",
        serverVersion: "v0.99.1"
    )
    let data = try JSONEncoder().encode([credential])
    let decoded = try JSONDecoder().decode([ServerCredential].self, from: data)
    #expect(decoded == [credential])
    #expect(decoded[0].gateway.webSocketURL.scheme == "wss")
    #expect(decoded[0].workingDir == "/home/gabe/project")
}

@Test func sameHostOverHTTPAndTLSSharesNoIdentifier() {
    let plain = ServerCredential(
        host: "box.tailnet.ts.net",
        port: 443,
        token: "a",
        serverName: "jcode",
        serverVersion: "v1"
    )
    let tls = ServerCredential(
        host: "box.tailnet.ts.net",
        port: 443,
        security: .https,
        token: "b",
        serverName: "jcode",
        serverVersion: "v1"
    )
    #expect(plain.id != tls.id)
}

@Test func inMemoryStoreReplacesByID() {
    let store = InMemoryCredentialStore()
    var credential = ServerCredential(
        host: "box",
        port: 7643,
        token: "a",
        serverName: "jcode",
        serverVersion: "v1"
    )
    store.save(credential)
    credential.workingDir = "/tmp"
    store.save(credential)

    #expect(store.loadAll().count == 1)
    #expect(store.loadAll().first?.workingDir == "/tmp")
}
