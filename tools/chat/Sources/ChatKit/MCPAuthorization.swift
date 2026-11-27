import Foundation
import MCP
import Synchronization

typealias MCPBrowser = @Sendable (URL) async throws -> Void

/// Owns registration and token replacement as one Keychain record. The SDK
/// calls this hook on 401 instead of clearing its token before trying refresh.
final class MCPAuthorization: HTTPClientAuthorizer {
    let maxAuthorizationAttempts = 2
    private let endpoint: URL
    private let store: any MCPCredentialStore
    private let browser: MCPBrowser?
    private let signInAgain: Bool
    private let startCallback: MCPCallback.Start
    private let login: Mutex<MCPLogin?>
    private struct Challenge {
        var attempted = false
        var cancelled = false
        var task: Task<Bool, any Error>?
    }
    private let challenge = Mutex(Challenge())
    private let failure = Mutex<MCPFailure?>(nil)
    private let discovery = DefaultOAuthMetadataDiscovery()

    init(endpoint: URL, store: any MCPCredentialStore, browser: MCPBrowser?, signInAgain: Bool,
         startCallback: @escaping MCPCallback.Start) throws {
        self.endpoint = endpoint
        self.store = store
        self.browser = browser
        self.signInAgain = signInAgain
        self.startCallback = startCallback
        do { login = Mutex(try store.load(endpoint)) }
        catch { throw MCPFailure.credentials }
    }

    var authorizationFailure: MCPFailure? { failure.withLock { $0 } }

    func validateEndpointSecurity(for endpoint: URL) throws {
        guard self.endpoint == endpoint, MCPAddress.valid(endpoint) else { throw MCPFailure.address }
        if challenge.withLock({ $0.cancelled }) { throw CancellationError() }
    }

    func authorizationHeader(for endpoint: URL) -> String? {
        guard endpoint == self.endpoint else { return nil }
        let hasTried = challenge.withLock { $0.attempted }
        if signInAgain && !hasTried { return nil }
        guard let token = login.withLock({ $0?.token }), !token.isExpired() else { return nil }
        return "Bearer " + token.value
    }

    func handleChallenge(statusCode: Int, headers: [String: String], endpoint: URL,
                         operationKey: String?, session: URLSession) async throws -> Bool {
        try validateEndpointSecurity(for: endpoint)
        guard statusCode == 401 else { return false }
        let task = challenge.withLock { challenge -> Task<Bool, any Error>? in
            guard !challenge.attempted, !challenge.cancelled else { return nil }
            challenge.attempted = true
            let task = Task { try await self.authorize(headers: headers) }
            challenge.task = task
            return task
        }
        guard let task else { return false }
        defer { challenge.withLock { $0.task = nil } }
        return try await task.value
    }

    func cancel() {
        let task = challenge.withLock { challenge in
            challenge.cancelled = true
            return challenge.task
        }
        task?.cancel()
    }

    func waitForAuthorization() async {
        let task = challenge.withLock { $0.task }
        _ = await task?.result
    }

    private func authorize(headers: [String: String]) async throws -> Bool {
        try Task.checkCancellation()
        let current = login.withLock { $0 }
        do {
            if !signInAgain, let current, let refreshToken = current.token?.refreshToken {
                try await refresh(current, refreshToken: refreshToken)
            } else {
                guard let browser, signInAgain || current?.token == nil else { throw MCPFailure.signInRequired }
                try await signIn(current, headers: headers, browser: browser)
            }
            return true
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            let safe = error as? MCPFailure ?? .signIn
            failure.withLock { $0 = safe }
            throw safe
        }
    }

    private func refresh(_ previous: MCPLogin, refreshToken: String) async throws {
        let token: OAuthAccessToken
        do {
            token = try await requestToken(registration: previous.registration, fields: [
                "grant_type": "refresh_token", "refresh_token": refreshToken,
            ], fallbackToken: previous.token)
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            throw MCPFailure.refresh
        }
        var updated = previous
        updated.token = token
        try save(updated)
    }

    private func signIn(_ previous: MCPLogin?, headers: [String: String], browser: @escaping MCPBrowser) async throws {
        let pending = store.attempts.registration(for: endpoint)
        let challenge = DefaultOAuthWWWAuthenticateParser().parseBearer(from: headers)
        let resource = try await discoverResource(challenge)
        let issuers = resource.authorizationServers.prefix(8)
        let issuer = try preferredIssuer(previous?.registration.issuer ?? pending?.issuer, among: issuers)
        let metadata = try await discoverIssuer(issuer)
        let scope = challenge?.scope ?? resource.scopesSupported?.joined(separator: " ")
        // RFC 7591 assigns client credentials to the issuer and gives issued
        // secrets an expiry. SDK 0.12.1 reuses the registration until expiry.
        let usable = [pending, previous?.registration].compactMap { $0 }.first {
            $0.issuer == issuer && ($0.secretExpiry ?? .distantFuture) > Date()
        }
        let state = UUID().uuidString
        let callback = try await startCallback(state, usable?.redirect)
        let (registration, code, verifier) = try await callback.withListener {
            try Task.checkCancellation()
            var registration: MCPRegistration
            if let usable {
                registration = usable
                registration.tokenEndpoint = metadata.tokenEndpoint
            } else {
                // A POST can succeed at the issuer even after the user cancels.
                // Let the bounded HTTP request finish so we can keep its reply.
                registration = try await Task {
                    try await self.register(issuer, metadata: metadata, redirect: callback.redirect)
                }.value
                store.attempts.remember(registration, for: endpoint)
            }
            // Keep a valid prior grant paired with its own registration until a
            // replacement succeeds. During a first sign-in, save registration now.
            if previous?.token == nil, usable == nil || usable != previous?.registration {
                try save(MCPLogin(registration: registration))
                store.attempts.remember(nil, for: endpoint)
            }

            let verifier = UUID().uuidString + UUID().uuidString
            let codeChallenge = try PKCE.makeChallenge(from: verifier)
            let fields = [
                "client_id": registration.clientID, "redirect_uri": callback.redirect.absoluteString,
                "response_type": "code", "state": state, "code_challenge": codeChallenge,
                "code_challenge_method": "S256", "resource": try discovery.canonicalResourceURI(from: endpoint).absoluteString,
            ]
            guard var components = URLComponents(url: metadata.authorizationEndpoint, resolvingAgainstBaseURL: false) else {
                throw MCPFailure.discovery
            }
            var items = fields.map { URLQueryItem(name: $0.key, value: $0.value) }
            if let scope, !scope.isEmpty { items.append(URLQueryItem(name: "scope", value: scope)) }
            let existing = components.queryItems ?? []
            guard !existing.contains(where: { entry in items.contains { $0.name == entry.name } }) else {
                throw MCPFailure.discovery
            }
            components.queryItems = existing + items
            components.percentEncodedQuery = components.percentEncodedQuery?.replacingOccurrences(of: "+", with: "%2B")
            guard let authorizationURL = components.url else { throw MCPFailure.discovery }
            let code = try await callback.waitForCode(opening: authorizationURL, with: browser)
            return (registration, code, verifier)
        }
        try Task.checkCancellation()
        let token = try await requestToken(registration: registration, fields: [
            "grant_type": "authorization_code", "code": code, "code_verifier": verifier,
            "redirect_uri": callback.redirect.absoluteString,
        ])
        try save(MCPLogin(registration: registration, token: token))
        store.attempts.remember(nil, for: endpoint)
    }

    private func preferredIssuer(_ previous: URL?, among issuers: ArraySlice<URL>) throws -> URL {
        if let previous {
            guard issuers.contains(previous) else { throw MCPFailure.issuerChanged }
            return previous
        }
        guard let issuer = issuers.first(where: isOAuthURL) else { throw MCPFailure.discovery }
        return issuer
    }

    private func discoverResource(_ challenge: OAuthBearerChallenge?) async throws -> OAuthProtectedResourceServerMetadata {
        let candidates = [challenge?.resourceMetadataURL].compactMap { $0 } +
            discovery.protectedResourceMetadataURLs(for: endpoint)
        for url in candidates where isOAuthURL(url) {
            try Task.checkCancellation()
            if let resource = try? await decode(OAuthProtectedResourceServerMetadata.self, request: URLRequest(url: url)),
               let resourceURL = URL(string: resource.resource),
               discovery.protectedResourceMatches(resource: resourceURL, endpoint: endpoint),
               !resource.authorizationServers.isEmpty {
                return resource
            }
        }
        try Task.checkCancellation()
        throw MCPFailure.discovery
    }

    private func discoverIssuer(_ issuer: URL) async throws -> IssuerMetadata {
        for url in discovery.authorizationServerMetadataURLs(for: issuer) where isOAuthURL(url) {
            try Task.checkCancellation()
            if let metadata = try? await decode(IssuerMetadata.self, request: URLRequest(url: url)),
               metadata.issuer == issuer, metadata.codeChallengeMethodsSupported.contains("S256"),
               isOAuthURL(metadata.authorizationEndpoint), isOAuthURL(metadata.tokenEndpoint) {
                return metadata
            }
        }
        try Task.checkCancellation()
        throw MCPFailure.discovery
    }

    private func isOAuthURL(_ url: URL) -> Bool {
        guard MCPAddress.valid(url, allowsQuery: true) else { return false }
        if url.scheme == "http" { return endpoint.scheme == "http" && url.host == endpoint.host }
        guard let host = url.host else { return false }
        let literalIP = host.contains(":") || host.utf8.allSatisfy { $0 == 46 || (48...57).contains($0) }
        return !literalIP || !OAuthURLValidator().isPrivateIPHost(host)
    }

    private func register(_ issuer: URL, metadata: IssuerMetadata, redirect: URL) async throws -> MCPRegistration {
        guard let registration = metadata.registrationEndpoint, isOAuthURL(registration) else {
            throw MCPFailure.registration
        }
        let supported = metadata.tokenEndpointAuthMethodsSupported ?? ["none"]
        guard let method = ["none", "client_secret_basic", "client_secret_post"].first(where: supported.contains) else {
            throw MCPFailure.registration
        }
        var request = URLRequest(url: registration)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: [
            "client_name": "OpenUI Chat", "token_endpoint_auth_method": method,
            "redirect_uris": [redirect.absoluteString], "response_types": ["code"],
            "grant_types": ["authorization_code", "refresh_token"],
        ])
        let reply: RegistrationReply
        do { reply = try await decode(RegistrationReply.self, request: request) }
        catch is CancellationError { throw CancellationError() }
        catch { throw MCPFailure.registration }
        let registeredMethod = reply.tokenEndpointAuthMethod ?? method
        guard registeredMethod == method,
              !reply.clientID.isEmpty,
              reply.redirectURIs?.contains(redirect.absoluteString) != false,
              method == "none" || reply.clientSecret?.isEmpty == false else { throw MCPFailure.registration }
        return MCPRegistration(issuer: issuer, tokenEndpoint: metadata.tokenEndpoint, redirect: redirect,
                               clientID: reply.clientID, method: method, clientSecret: reply.clientSecret,
                               secretExpiry: reply.clientSecretExpiresAt.flatMap { $0 == 0 ? nil : Date(timeIntervalSince1970: $0) })
    }

    private func requestToken(registration: MCPRegistration, fields: [String: String],
                              fallbackToken: OAuthAccessToken? = nil) async throws -> OAuthAccessToken {
        guard isOAuthURL(registration.tokenEndpoint) else { throw MCPFailure.discovery }
        var request = URLRequest(url: registration.tokenEndpoint)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        var body = fields
        body["resource"] = try discovery.canonicalResourceURI(from: endpoint).absoluteString
        switch registration.method {
        case "none":
            body["client_id"] = registration.clientID
        case "client_secret_post":
            body["client_id"] = registration.clientID
            body["client_secret"] = registration.clientSecret
        case "client_secret_basic":
            guard let secret = registration.clientSecret else { throw MCPFailure.credentials }
            let value = try Self.formValue(registration.clientID) + ":" + Self.formValue(secret)
            request.setValue("Basic " + Data(value.utf8).base64EncodedString(), forHTTPHeaderField: "Authorization")
        default:
            throw MCPFailure.credentials
        }
        request.httpBody = try Data(body.map { try "\($0.key)=\(Self.formValue($0.value))" }.joined(separator: "&").utf8)
        let response = try await decode(TokenReply.self, request: request)
        guard response.tokenType.lowercased() == "bearer", !response.accessToken.isEmpty,
              response.accessToken.utf8.count <= 16 << 10,
              response.accessToken.utf8.allSatisfy({ (33...126).contains($0) }),
              (response.expiresIn ?? 1) > 0 else { throw MCPFailure.signIn }
        let scopes = response.scope.map { DefaultOAuthScopeSelector().parseScopeString($0) }
        return OAuthAccessToken(value: response.accessToken, tokenType: "Bearer",
                                expiresAt: response.expiresIn.map { Date().addingTimeInterval($0) },
                                scopes: scopes ?? fallbackToken?.scopes ?? [],
                                authorizationServer: registration.issuer,
                                refreshToken: response.refreshToken ?? fallbackToken?.refreshToken,
                                clientID: registration.clientID)
    }

    private static func formValue(_ value: String) throws -> String {
        let safe = CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
        guard let encoded = value.addingPercentEncoding(withAllowedCharacters: safe) else { throw MCPFailure.signIn }
        return encoded
    }

    private func decode<T: Decodable>(_ type: T.Type, request: URLRequest) async throws -> T {
        var request = request
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        let (body, response) = try await MCPHTTP.read(request)
        try Task.checkCancellation()
        guard (200..<300).contains(response.statusCode) else { throw MCPFailure.signIn }
        return try JSONDecoder().decode(type, from: body)
    }

    private func save(_ updated: MCPLogin) throws {
        try Task.checkCancellation()
        do {
            try store.save(updated, for: endpoint)
            login.withLock { $0 = updated }
        } catch {
            throw MCPFailure.credentials
        }
    }
}

private struct IssuerMetadata: Decodable {
    let issuer: URL
    let authorizationEndpoint: URL
    let tokenEndpoint: URL
    let registrationEndpoint: URL?
    let tokenEndpointAuthMethodsSupported: [String]?
    let codeChallengeMethodsSupported: [String]

    enum CodingKeys: String, CodingKey {
        case issuer
        case authorizationEndpoint = "authorization_endpoint"
        case tokenEndpoint = "token_endpoint"
        case registrationEndpoint = "registration_endpoint"
        case tokenEndpointAuthMethodsSupported = "token_endpoint_auth_methods_supported"
        case codeChallengeMethodsSupported = "code_challenge_methods_supported"
    }
}

private struct RegistrationReply: Decodable {
    let clientID: String
    let clientSecret: String?
    let clientSecretExpiresAt: TimeInterval?
    let redirectURIs: [String]?
    let tokenEndpointAuthMethod: String?

    enum CodingKeys: String, CodingKey {
        case clientID = "client_id"
        case clientSecret = "client_secret"
        case clientSecretExpiresAt = "client_secret_expires_at"
        case redirectURIs = "redirect_uris"
        case tokenEndpointAuthMethod = "token_endpoint_auth_method"
    }
}

private struct TokenReply: Decodable {
    let accessToken: String
    let tokenType: String
    let expiresIn: TimeInterval?
    let refreshToken: String?
    let scope: String?

    enum CodingKeys: String, CodingKey {
        case accessToken = "access_token"
        case tokenType = "token_type"
        case expiresIn = "expires_in"
        case refreshToken = "refresh_token"
        case scope
    }
}
