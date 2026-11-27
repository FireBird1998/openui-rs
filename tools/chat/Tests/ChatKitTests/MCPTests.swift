import AppKit
import Foundation
import SwiftUI
import Synchronization
import Testing
@testable import ChatKit

// The occupied-port test checks the process's listening sockets. The MCP and
// Settings lifecycle cases run serially so their listeners cannot overlap.
@Suite(.serialized, .timeLimit(.minutes(1))) @MainActor struct MCPTests {
    @Test func listsEveryToolPageAndRemembersSwitches() async throws {
        let server = try await MCPFixture.start()
        defer { server.stop() }
        let testSession = URLSession(configuration: MCPHTTP.configuration())
        defer { testSession.invalidateAndCancel() }
        let (_, standaloneGET) = try await testSession.data(from: server.endpoint)
        #expect((standaloneGET as? HTTPURLResponse)?.statusCode == 405)

        let preferences = MemoryPreferences()
        let settings = MCPSettings(service: service(), preferences: preferences)
        settings.address = server.endpoint.absoluteString
        await settings.addServer()
        let connected = try #require(settings.servers.first)
        #expect(connected.tools.map(\.name) == ["lookup", "search"])
        #expect(connected.tools.map(\.enabled) == [true, true])
        settings.setEnabled(false, server: server.endpoint, tool: "lookup")

        let relaunched = MCPSettings(service: service(), preferences: preferences)
        await relaunched.refresh(server.endpoint)
        let tools = try #require(relaunched.servers.first?.tools)
        #expect(tools.first?.enabled == false)
        #expect(tools.last?.enabled == true)
        #expect(server.getsAtMCP.count == 1)
    }

    @Test func signsInThroughLoopbackAndRemembersRegistration() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        let settings = MCPSettings(service: service(credentials, browser: server.openBrowser),
                                   preferences: MemoryPreferences())
        settings.address = server.endpoint.absoluteString
        await settings.addServer()
        #expect(settings.message == nil)
        #expect(settings.servers.first?.tools.count == 2)
        #expect(server.registrations.count == 1)
        #expect(server.authorizations.count == 1)
        #expect(server.codeExchanges.count == 1)
        #expect(try await server.callbackIsClosed())

        let relaunched = service(credentials)
        #expect(try await relaunched.tools(at: server.endpoint).count == 2)
        #expect(server.registrations.count == 1)
        #expect(server.authorizations.count == 1)
    }

    @Test func aFailedBrowserLaunchClosesTheListenerAndRetainsRegistrationForRetry() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()

        await #expect(throws: MCPFailure.browser) {
            try await self.service(credentials).tools(at: server.endpoint)
        }
        #expect(server.registrations.count == 1)
        #expect(server.codeExchanges.count == 0)
        #expect(try await server.callbackIsClosed())

        let retry = service(credentials, browser: server.openBrowser)
        #expect(try await retry.tools(at: server.endpoint).count == 2)
        #expect(server.registrations.count == 1)
        #expect(server.codeExchanges.count == 1)
    }

    enum StartingStore: CaseIterable {
        case empty, registrationOnly, priorGrant
    }

    enum FailedSignInStep: CaseIterable {
        case browser, callbackTimeout, denial, codeExchange, shutdownDeadline, cancelWhileWaiting

        var error: MCPFailure? {
            switch self {
            case .browser: .browser
            case .callbackTimeout: .signInTimeout
            case .denial: .denied
            case .codeExchange: .signIn
            case .shutdownDeadline: .listenerShutdownTimeout
            case .cancelWhileWaiting: nil
            }
        }
    }

    @Test(arguments: FailedSignInStep.allCases, StartingStore.allCases)
    func failureEndStates(_ step: FailedSignInStep, _ start: StartingStore) async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        try await prepare(start, on: server, in: credentials)
        let saved = try credentials.snapshot(server.endpoint)
        let registrations = server.registrations.count
        let exchanges = server.codeExchanges.count
        let browserOpened = Flag()
        server.denyAuthorization(step == .denial)
        server.refuseCode(step == .codeExchange)
        let client = MCPService(store: credentials, startCallback: { state, preferred in
            switch step {
            case .callbackTimeout:
                return try await MCPCallback.start(state: state, preferred: preferred, timeout: .milliseconds(100))
            case .shutdownDeadline:
                return try await MCPCallback.start(state: state, preferred: preferred) { port in
                    LoopbackHTTPListener(port: port, shutdown: { _, _ in })
                }
            default:
                return try await MCPCallback.start(state: state, preferred: preferred)
            }
        }, browser: { url in
            #expect(try server.callbackAcceptsTCP())
            browserOpened.set()
            switch step {
            case .browser, .shutdownDeadline: throw MCPFailure.browser
            case .denial, .codeExchange: try await server.openBrowser(url)
            case .callbackTimeout, .cancelWhileWaiting: break
            }
        })

        if let error = step.error {
            await #expect(throws: error) {
                try await client.tools(at: server.endpoint, signInAgain: start == .priorGrant)
            }
        } else {
            let pending = Task { try await client.tools(at: server.endpoint, signInAgain: start == .priorGrant) }
            defer { pending.cancel() }
            try #require(await eventually { browserOpened.isSet })
            #expect(try await server.callbackResponds())
            pending.cancel()
            await #expect(throws: CancellationError.self) { try await pending.value }
        }

        try expectCallbackReleased(server)
        #expect(browserOpened.isSet)
        #expect(server.registrations.count - registrations == (start == .empty ? 1 : 0))
        #expect(server.codeExchanges.count - exchanges == (step == .codeExchange ? 1 : 0))
        let expected = try saved ?? registrationOnlySnapshot(server)
        #expect(try credentials.snapshot(server.endpoint) == expected)
    }

    @Test func anOccupiedCallbackPortPreservesTheRegistration() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        await #expect(throws: MCPFailure.browser) {
            try await self.service(credentials).tools(at: server.endpoint)
        }
        let saved = try #require(try credentials.snapshot(server.endpoint))
        let redirect = try #require(credentials.load(server.endpoint)?.registration.redirect)
        let port = try #require(redirect.port.flatMap(UInt16.init(exactly:)))
        let occupyingListener = LoopbackHTTPListener(port: port)
        defer { occupyingListener.stop() }
        _ = try occupyingListener.start()

        let listeningBeforeAttempt = try MCPFixture.listeningTCPSockets()
        try #require(listeningBeforeAttempt.contains { $0.port == port })
        let requestedPorts = Mutex<[UInt16]>([])
        let client = MCPService(store: credentials, startCallback: { state, preferred in
            do {
                return try await MCPCallback.start(state: state, preferred: preferred) { port in
                    requestedPorts.withLock { $0.append(port) }
                    return LoopbackHTTPListener(port: port)
                }
            } catch {
                let listening = try? MCPFixture.listeningTCPSockets()
                #expect(listening?.subtracting(listeningBeforeAttempt) == [])
                throw error
            }
        }, browser: server.openBrowser)
        await #expect(throws: MCPFailure.listener) {
            try await client.tools(at: server.endpoint)
        }
        #expect(requestedPorts.withLock { $0 } == [port])
        let newListeners = try MCPFixture.listeningTCPSockets().subtracting(listeningBeforeAttempt)
        #expect(newListeners.isEmpty)
        #expect(try credentials.snapshot(server.endpoint) == saved)
        #expect(try server.callbackAcceptsTCP())
        #expect(server.registrations.count == 1)
        #expect(server.authorizations.count == 0)
        #expect(server.codeExchanges.count == 0)

        try await occupyingListener.stopAndWait()
        try expectCallbackReleased(server)
        #expect(try await client.tools(at: server.endpoint).count == 2)
        #expect(credentials.load(server.endpoint)?.registration.redirect == redirect)
        #expect(server.registrations.count == 1)
        #expect(server.codeExchanges.count == 1)
    }

    @Test func retryAfterFirstCredentialWriteFailsReusesTheRegisteredClient() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        credentials.failWrite(1)
        let saved = try credentials.snapshot(server.endpoint)

        await #expect(throws: MCPFailure.credentials) {
            try await self.service(credentials, browser: server.openBrowser).tools(at: server.endpoint)
        }
        try expectCallbackReleased(server)
        #expect(try credentials.snapshot(server.endpoint) == saved)
        #expect(server.registrations.count == 1)
        #expect(server.authorizations.count == 0)
        #expect(server.codeExchanges.count == 0)
        let issued = try server.issuedRegistration()

        credentials.failNextWrite()
        await #expect(throws: MCPFailure.credentials) {
            try await self.service(credentials, browser: server.openBrowser).tools(at: server.endpoint)
        }
        try expectCallbackReleased(server)
        #expect(try credentials.snapshot(server.endpoint) == saved)
        #expect(server.registrations.count == 1)
        #expect(server.authorizations.count == 0)
        #expect(server.codeExchanges.count == 0)

        #expect(try await service(credentials, browser: server.openBrowser).tools(at: server.endpoint).count == 2)
        #expect(server.registrations.count == 1)
        #expect(server.codeExchanges.count == 1)
        #expect(credentials.load(server.endpoint)?.registration == issued)
        #expect(credentials.load(server.endpoint)?.token != nil)
    }

    @Test(arguments: [StartingStore.empty, .registrationOnly])
    func aFailedGrantWritePreservesTheRegistration(_ start: StartingStore) async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        try await prepare(start, on: server, in: credentials)
        if start == .registrationOnly {
            credentials.failNextWrite()
        } else {
            credentials.failWrite(2)
        }
        let saved = try credentials.snapshot(server.endpoint)
        let exchanges = server.codeExchanges.count

        await #expect(throws: MCPFailure.credentials) {
            try await self.service(credentials, browser: server.openBrowser).tools(at: server.endpoint)
        }
        try expectCallbackReleased(server)
        let expected = try saved ?? registrationOnlySnapshot(server)
        #expect(try credentials.snapshot(server.endpoint) == expected)
        #expect(server.registrations.count == 1)
        #expect(server.codeExchanges.count == exchanges + 1)

        #expect(try await service(credentials, browser: server.openBrowser).tools(at: server.endpoint).count == 2)
        #expect(server.registrations.count == 1)
        #expect(server.codeExchanges.count == exchanges + 2)
        #expect(try credentials.load(server.endpoint)?.registration == server.issuedRegistration())
        #expect(credentials.load(server.endpoint)?.token != nil)
    }

    enum ShutdownFault: CaseIterable {
        case missingAcknowledgement, missingClose
    }

    @Test(arguments: ShutdownFault.allCases)
    func aStalledListenerShutdownReportsTheDeadlineAndReleasesThePort(_ fault: ShutdownFault) async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        let browserFailed = Mutex<ContinuousClock.Instant?>(nil)
        let acceptsTCPAfterGracefulRequest = Mutex<Bool?>(nil)
        let client = MCPService(store: credentials, startCallback: { state, preferred in
            try await MCPCallback.start(state: state, preferred: preferred) { port in
                LoopbackHTTPListener(port: port, shutdown: { close, _ in
                    if fault == .missingAcknowledgement { close() }
                    acceptsTCPAfterGracefulRequest.withLock { $0 = try? server.callbackAcceptsTCP() }
                })
            }
        }, browser: { _ in
            #expect(try server.callbackAcceptsTCP())
            browserFailed.withLock { $0 = .now }
            throw MCPFailure.browser
        })

        await #expect(throws: MCPFailure.listenerShutdownTimeout) {
            try await client.tools(at: server.endpoint)
        }
        let elapsed = try #require(browserFailed.withLock { $0 }).duration(to: .now)
        #expect(elapsed >= .seconds(1) && elapsed < .seconds(8))
        #expect(acceptsTCPAfterGracefulRequest.withLock { $0 } == (fault == .missingClose))
        #expect(try !server.callbackAcceptsTCP())
        #expect(server.registrations.count == 1)
        #expect(server.codeExchanges.count == 0)
        #expect(credentials.load(server.endpoint)?.token == nil)

        // Immediate reuse proves the port is free to bind as well as refusing TCP.
        let retry = service(credentials, browser: server.openBrowser)
        #expect(try await retry.tools(at: server.endpoint).count == 2)
        #expect(server.registrations.count == 1)
    }

    @Test func aCallbackTimeoutClosesThePortAndRetainsTheRegistration() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        let atLaunch = Mutex<Data?>(nil)
        let client = MCPService(store: credentials, startCallback: { state, preferred in
            try await MCPCallback.start(state: state, preferred: preferred, timeout: .milliseconds(100))
        }, browser: { _ async throws in
            let acceptsTCP = try server.callbackAcceptsTCP()
            #expect(acceptsTCP)
            atLaunch.withLock { $0 = try? credentials.snapshot(server.endpoint) }
        })

        await #expect(throws: MCPFailure.signInTimeout) { try await client.tools(at: server.endpoint) }
        #expect(try !server.callbackAcceptsTCP())
        let registrationOnly = try #require(atLaunch.withLock { $0 })
        #expect(try credentials.snapshot(server.endpoint) == registrationOnly)
        #expect(server.codeExchanges.count == 0)
        #expect(try await service(credentials, browser: server.openBrowser).tools(at: server.endpoint).count == 2)
        #expect(server.registrations.count == 1)
    }

    @Test(arguments: StartingStore.allCases)
    func aSecondSignInSharingTheStoreIsBusyBeforeAndAfterRegistration(_ start: StartingStore) async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        try await prepare(start, on: server, in: credentials)
        let beforeBinding = try credentials.snapshot(server.endpoint)
        let registrations = server.registrations.count
        let exchanges = server.codeExchanges.count
        let signInAgain = start == .priorGrant
        let readyToBind = Flag()
        let launched = Flag()
        let bind = AsyncStream<Void>.makeStream()
        let firstStarts = Counter()
        let secondStarts = Counter()
        let first = MCPService(store: credentials, startCallback: { state, preferred in
            firstStarts.add(1)
            readyToBind.set()
            var iterator = bind.stream.makeAsyncIterator()
            _ = await iterator.next()
            return try await MCPCallback.start(state: state, preferred: preferred)
        }, browser: { _ in launched.set() })
        let second = MCPService(store: credentials, startCallback: { state, preferred in
            secondStarts.add(1)
            return try await MCPCallback.start(state: state, preferred: preferred)
        }, browser: server.openBrowser)
        let pending = Task { try await first.tools(at: server.endpoint, signInAgain: signInAgain) }
        defer {
            pending.cancel()
            bind.continuation.finish()
        }
        try #require(await eventually { readyToBind.isSet })
        await #expect(throws: MCPFailure.busy) { try await first.tools(at: server.endpoint, signInAgain: signInAgain) }
        #expect(firstStarts.count == 1)
        await #expect(throws: MCPFailure.busy) { try await second.tools(at: server.endpoint, signInAgain: signInAgain) }
        if beforeBinding != nil { try expectCallbackReleased(server) }
        #expect(secondStarts.count == 0)
        #expect(try credentials.snapshot(server.endpoint) == beforeBinding)
        #expect(server.registrations.count == registrations)
        bind.continuation.finish()

        try #require(await eventually { launched.isSet })
        let saved = try #require(try credentials.snapshot(server.endpoint))
        let afterBinding = try beforeBinding ?? registrationOnlySnapshot(server)
        #expect(saved == afterBinding)
        let afterRegistration = registrations + (start == .empty ? 1 : 0)
        #expect(server.registrations.count == afterRegistration)
        await #expect(throws: MCPFailure.busy) { try await second.tools(at: server.endpoint, signInAgain: signInAgain) }
        // The active listener belongs to the first attempt.
        #expect(try server.callbackAcceptsTCP())
        #expect(secondStarts.count == 0)
        #expect(server.registrations.count == afterRegistration)
        #expect(try credentials.snapshot(server.endpoint) == saved)
        pending.cancel()
        await #expect(throws: CancellationError.self) { try await pending.value }
        try expectCallbackReleased(server)
        #expect(try credentials.snapshot(server.endpoint) == saved)
        #expect(server.codeExchanges.count == exchanges)

        #expect(try await second.tools(at: server.endpoint, signInAgain: signInAgain).count == 2)
        #expect(server.registrations.count == 1)
        #expect(secondStarts.count == 1)
    }

    @Test(arguments: StartingStore.allCases)
    func cancellingRegistrationClosesThePortAndRetainsTheReply(_ start: StartingStore) async throws {
        let holdRegistration = Mutex(false)
        let registered = Flag()
        let reply = DispatchSemaphore(value: 0)
        let server = try await MCPFixture.start(protected: true) {
            let hold = holdRegistration.withLock { hold in
                defer { hold = false }
                return hold
            }
            guard hold else { return }
            registered.set()
            _ = reply.wait(timeout: .now() + .seconds(5))
        }
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        try await prepare(start, on: server, in: credentials, expiredRegistration: true)
        let saved = try credentials.snapshot(server.endpoint)
        let registrations = server.registrations.count
        let authorizations = server.authorizations.count
        let signInAgain = start == .priorGrant
        holdRegistration.withLock { $0 = true }
        let first = service(credentials)
        let pending = Task { try await first.tools(at: server.endpoint, signInAgain: signInAgain) }
        defer {
            pending.cancel()
            reply.signal()
        }
        try #require(await eventually { registered.isSet })
        #expect(try server.callbackAcceptsTCP())

        pending.cancel()
        try #require(await eventually { (try? server.callbackAcceptsTCP()) == false })
        #expect(try credentials.snapshot(server.endpoint) == saved)
        let retry = service(credentials, browser: server.openBrowser)
        await #expect(throws: MCPFailure.busy) { try await retry.tools(at: server.endpoint, signInAgain: signInAgain) }
        reply.signal()
        await #expect(throws: CancellationError.self) { try await pending.value }
        try expectCallbackReleased(server)
        #expect(try credentials.snapshot(server.endpoint) == saved)
        #expect(server.registrations.count == registrations + 1)
        #expect(server.authorizations.count == authorizations)
        let issued = try server.issuedRegistration()

        #expect(try await retry.tools(at: server.endpoint, signInAgain: signInAgain).count == 2)
        #expect(server.registrations.count == registrations + 1)
        #expect(credentials.load(server.endpoint)?.registration == issued)
    }

    @Test func aFailedReplacementSaveKeepsTheGrantAndReusesItsReplacementRegistration() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        try await prepare(.priorGrant, on: server, in: credentials, expiredRegistration: true)
        let saved = try #require(try credentials.snapshot(server.endpoint))
        credentials.failNextWrite()

        await #expect(throws: MCPFailure.credentials) {
            try await self.service(credentials, browser: server.openBrowser).tools(at: server.endpoint, signInAgain: true)
        }
        try expectCallbackReleased(server)
        #expect(try credentials.snapshot(server.endpoint) == saved)
        #expect(server.registrations.count == 2)
        let issued = try server.issuedRegistration()

        credentials.failNextWrite()
        await #expect(throws: MCPFailure.credentials) {
            try await self.service(credentials, browser: server.openBrowser).tools(at: server.endpoint, signInAgain: true)
        }
        try expectCallbackReleased(server)
        #expect(try credentials.snapshot(server.endpoint) == saved)
        #expect(server.registrations.count == 2)
        #expect(server.codeExchanges.count == 3)

        #expect(try await service(credentials, browser: server.openBrowser)
            .tools(at: server.endpoint, signInAgain: true).count == 2)
        #expect(server.registrations.count == 2)
        #expect(server.codeExchanges.count == 4)
        #expect(credentials.load(server.endpoint)?.registration == issued)
    }

    @Test func aFailedBrowserLaunchDuringSignInAgainPreservesTheSavedGrant() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        _ = try await service(credentials, browser: server.openBrowser).tools(at: server.endpoint)
        let saved = try #require(try credentials.snapshot(server.endpoint))
        await #expect(throws: MCPFailure.browser) {
            try await self.service(credentials).tools(at: server.endpoint, signInAgain: true)
        }
        #expect(try credentials.snapshot(server.endpoint) == saved)
        #expect(try !server.callbackAcceptsTCP())
        #expect(server.registrations.count == 1)
        #expect(server.codeExchanges.count == 1)
        #expect(try await service(credentials).tools(at: server.endpoint).count == 2)
    }

    @Test(arguments: [MCPFailure.denied, .signIn])
    func providerRejectionClosesThePortAndRetainsTheRegistration(_ failure: MCPFailure) async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        let client = service(credentials, browser: server.openBrowser)
        server.denyAuthorization(failure == .denied)
        server.refuseCode(failure == .signIn)
        await #expect(throws: failure) { try await client.tools(at: server.endpoint) }
        #expect(try !server.callbackAcceptsTCP())
        #expect(credentials.load(server.endpoint)?.registration != nil)
        #expect(credentials.load(server.endpoint)?.token == nil)
        #expect(server.codeExchanges.count == (failure == .denied ? 0 : 1))
        server.denyAuthorization(false)
        server.refuseCode(false)
        #expect(try await client.tools(at: server.endpoint).count == 2)
        #expect(server.registrations.count == 1)
    }

    @Test func refreshAfterRelaunchUsesTheRegisteredClientAndRotatesTheToken() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        _ = try await service(credentials, browser: server.openBrowser).tools(at: server.endpoint)
        server.expireAccessToken()
        #expect(try await service(credentials).tools(at: server.endpoint).count == 2)
        #expect(server.refreshes.count == 1)
        #expect(server.registrations.count == 1)
        #expect(server.authorizations.count == 1)
        server.expireAccessToken()
        #expect(try await service(credentials).tools(at: server.endpoint).count == 2)
        #expect(server.refreshes.count == 2)
    }

    @Test func rejectedRefreshKeepsTheSignInAndToolsForAnotherTry() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        let callbackStarts = Counter()
        let client = MCPService(store: credentials, startCallback: { state, preferred in
            callbackStarts.add(1)
            return try await MCPCallback.start(state: state, preferred: preferred)
        }, browser: server.openBrowser)
        let settings = MCPSettings(service: client, preferences: MemoryPreferences())
        settings.address = server.endpoint.absoluteString
        await settings.addServer()
        let saved = try #require(try credentials.snapshot(server.endpoint))
        #expect(callbackStarts.count == 1)
        server.expireAccessToken()
        server.refuseRefresh(true)
        await settings.refresh(server.endpoint)
        try expectCallbackReleased(server)
        #expect(callbackStarts.count == 1)
        #expect(server.registrations.count == 1)
        #expect(server.refreshes.count == 1)
        #expect(settings.message == MCPFailure.refresh.localizedDescription)
        #expect(settings.servers.first?.tools.count == 2)
        #expect(server.authorizations.count == 1)
        #expect(try credentials.snapshot(server.endpoint) == saved)
        server.refuseRefresh(false)
        #expect(try await service(credentials).tools(at: server.endpoint).count == 2)
        #expect(server.refreshes.count == 2)
        #expect(server.registrations.count == 1)
    }

    @Test func signInAgainReusesTheRegistrationAfterRefreshIsRejected() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        let settings = MCPSettings(service: service(credentials, browser: server.openBrowser),
                                   preferences: MemoryPreferences())
        settings.address = server.endpoint.absoluteString
        await settings.addServer()
        server.expireAccessToken()
        server.refuseRefresh(true)
        await settings.refresh(server.endpoint)
        await settings.signInAgain(server.endpoint)
        #expect(settings.message == nil)
        #expect(settings.servers.first?.tools.count == 2)
        #expect(server.registrations.count == 1)
        #expect(server.authorizations.count == 2)
        #expect(server.refreshes.count == 1)
        #expect(try await server.callbackIsClosed())
    }

    @Test func duplicateNameAcrossServersRefusesTheSecondListingEvenIfTheFirstToolIsOff() async throws {
        let first = try await MCPFixture.start()
        defer { first.stop() }
        let second = try await MCPFixture.start()
        defer { second.stop() }
        let preferences = MemoryPreferences()
        let settings = MCPSettings(service: service(), preferences: preferences)
        settings.address = first.endpoint.absoluteString
        await settings.addServer()
        settings.setEnabled(false, server: first.endpoint, tool: "lookup")

        let relaunched = MCPSettings(service: service(), preferences: preferences)
        relaunched.address = second.endpoint.absoluteString
        await relaunched.addServer()
        #expect(relaunched.servers.map(\.address) == [first.endpoint])
        #expect(relaunched.message?.contains("lookup") == true)
        #expect(relaunched.message?.contains(first.endpoint.absoluteString) == true)
        #expect(relaunched.message?.contains(second.endpoint.absoluteString) == true)
    }

    @Test(arguments: StartingStore.allCases)
    func aRefusedProtectedAddDeletesItsCredentialRecord(_ start: StartingStore) async throws {
        let first = try await MCPFixture.start()
        defer { first.stop() }
        let refused = try await MCPFixture.start(protected: true)
        defer { refused.stop() }
        let credentials = MemoryMCPStore()
        try await prepare(start, on: refused, in: credentials)
        let registrations = refused.registrations.count
        let exchanges = refused.codeExchanges.count
        let callbackStarts = Counter()
        let client = MCPService(store: credentials, startCallback: { state, preferred in
            callbackStarts.add(1)
            return try await MCPCallback.start(state: state, preferred: preferred)
        }, browser: refused.openBrowser)
        let preferences = MemoryPreferences()
        let settings = MCPSettings(service: client, preferences: preferences)
        settings.address = first.endpoint.absoluteString
        await settings.addServer()
        settings.address = refused.endpoint.absoluteString
        await settings.addServer()

        try expectCallbackReleased(refused)
        let interactiveAttempts = start == .priorGrant ? 0 : 1
        #expect(callbackStarts.count == interactiveAttempts)
        #expect(refused.registrations.count - registrations == (start == .empty ? 1 : 0))
        #expect(refused.codeExchanges.count - exchanges == interactiveAttempts)
        #expect(try credentials.snapshot(refused.endpoint) == nil)
        #expect(settings.message == MCPFailure.duplicateTool("lookup", first.endpoint, refused.endpoint).localizedDescription)
        #expect(settings.servers.map(\.address) == [first.endpoint])
        #expect(MCPSettings(preferences: preferences).servers.map(\.address) == [first.endpoint])
        await settings.remove(first.endpoint)
        await settings.addServer()
        try expectCallbackReleased(refused)
        #expect(settings.servers.map(\.address) == [refused.endpoint])
        #expect(callbackStarts.count == interactiveAttempts + 1)
        #expect(refused.registrations.count == 1)
        #expect(refused.codeExchanges.count == exchanges + interactiveAttempts + 1)
    }

    @Test func removingASignedInServerDeletesItsRegistrationAndTokens() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let credentials = MemoryMCPStore()
        let preferences = MemoryPreferences()
        let settings = MCPSettings(service: service(credentials, browser: server.openBrowser), preferences: preferences)
        settings.address = server.endpoint.absoluteString
        await settings.addServer()
        try #require(settings.servers.first?.address == server.endpoint)
        let signedIn = credentials.load(server.endpoint)?.token != nil
        try #require(signedIn)

        await settings.remove(server.endpoint)

        let retainedCredentials = credentials.load(server.endpoint) != nil
        #expect(!retainedCredentials)
        #expect(settings.message == nil)
        #expect(settings.servers.isEmpty)
        #expect(MCPSettings(preferences: preferences).servers.isEmpty)
    }

    @Test func twoServersWithDistinctToolsCanBeConnected() async throws {
        let first = try await MCPFixture.start()
        defer { first.stop() }
        let second = try await MCPFixture.start(toolNames: ["fetch", "find"])
        defer { second.stop() }
        let settings = MCPSettings(service: service(), preferences: MemoryPreferences())
        for address in [first.endpoint, second.endpoint] {
            settings.address = address.absoluteString
            await settings.addServer()
        }
        #expect(settings.servers.map(\.address) == [first.endpoint, second.endpoint])
    }

    @Test func anOversizedResponseIsRefused() async throws {
        let server = try await MCPFixture.start(oversized: true)
        defer { server.stop() }
        await #expect(throws: MCPFailure.self) {
            try await self.service().tools(at: server.endpoint)
        }
    }

    @Test func theListenerIsBoundOnlyToIPv4Loopback() async throws {
        let callback = try await MCPCallback.start(state: "fixture-state", preferred: nil)
        defer { callback.stop() }
        let port = try #require(callback.redirect.port)
        // Inspect the actual listening socket. A request to 127.0.0.1 would also work on a wildcard bind.
        let lsof = Process()
        lsof.executableURL = URL(fileURLWithPath: "/usr/sbin/lsof")
        lsof.arguments = ["-nP", "-a", "-p", "\(getpid())", "-iTCP:\(port)", "-sTCP:LISTEN", "-Fn"]
        let output = Pipe()
        lsof.standardOutput = output
        try lsof.run()
        let sockets = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        lsof.waitUntilExit()
        #expect(lsof.terminationStatus == 0)
        #expect(sockets.split(separator: "\n").filter { $0.hasPrefix("n") } == ["n127.0.0.1:\(port)"])
    }

    @Test func theCallbackAcceptsAValidStateOnlyOnce() async throws {
        let callback = try await MCPCallback.start(state: "fixture-state", preferred: nil)
        defer { callback.stop() }
        let url = try #require(URL(string: callback.redirect.absoluteString + "?code=fixture-code&state=fixture-state"))
        let session = URLSession(configuration: MCPHTTP.configuration())
        defer { session.invalidateAndCancel() }
        let (_, first) = try await session.data(from: url)
        let (_, replay) = try await session.data(from: url)
        #expect((first as? HTTPURLResponse)?.statusCode == 200)
        #expect((replay as? HTTPURLResponse)?.statusCode == 400)
    }

    @Test func everySignInUsesAFreshState() async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let states = Mutex<[String]>([])
        let client = service { url in
            guard let state = URLComponents(url: url, resolvingAgainstBaseURL: false)?
                .queryItems?.first(where: { $0.name == "state" })?.value,
                !state.isEmpty else { throw MCPFailure.browser }
            states.withLock { $0.append(state) }
            try await server.openBrowser(url)
        }
        _ = try await client.tools(at: server.endpoint)
        _ = try await client.tools(at: server.endpoint, signInAgain: true)
        #expect(states.withLock { Set($0).count } == 2)
    }

    @Test func theToolLimitAppliesAcrossPages() async throws {
        let server = try await MCPFixture.start(toolNames: (0..<1025).map { "tool-\($0)" })
        defer { server.stop() }
        await #expect(throws: MCPFailure.toolLimit) {
            try await self.service().tools(at: server.endpoint)
        }
    }

    @Test func duplicateNamesAcrossPagesFromOneServerAreRefused() async throws {
        let server = try await MCPFixture.start(toolNames: ["lookup", "lookup"])
        defer { server.stop() }
        let settings = MCPSettings(service: service(), preferences: MemoryPreferences())
        settings.address = server.endpoint.absoluteString
        await settings.addServer()
        #expect(settings.servers.isEmpty)
        #expect(settings.message == MCPFailure.duplicateTool("lookup", server.endpoint, server.endpoint).localizedDescription)
    }

    @Test func mcpAndOAuthHTTPRequestsDoNotFollowRedirects() async throws {
        let server = LoopbackHTTPListener()
        defer { server.stop() }
        let origin = try server.start()
        let visited = Counter()
        server.respond { request in
            if request.target == "/start" {
                return .init(status: 307, headers: ["Location": origin.appending(path: "target").absoluteString])
            }
            visited.add(1)
            return .init(status: 200)
        }
        let (_, response) = try await MCPHTTP.read(URLRequest(url: origin.appending(path: "start")))
        #expect(response.statusCode == 307)
        #expect(visited.count == 0)
    }

    @Test func signInAgainRefusesAChangedIssuer() async throws {
        let original = try await MCPFixture.start(protected: true)
        defer { original.stop() }
        let replacement = try await MCPFixture.start(protected: true)
        defer { replacement.stop() }
        let client = service(browser: original.openBrowser)
        _ = try await client.tools(at: original.endpoint)
        original.advertiseIssuer(of: replacement)

        await #expect(throws: MCPFailure.issuerChanged) {
            try await client.tools(at: original.endpoint, signInAgain: true)
        }
        #expect(original.authorizations.count == 1)
        #expect(replacement.registrations.count == 0)
    }

    @Test func discoveryRefusesAPrivateAddressInOAuthMetadata() async throws {
        let privateURL = try #require(URL(string: "https://192.168.1.1/authorize"))
        let server = try await MCPFixture.start(protected: true, authorizationEndpoint: privateURL)
        defer { server.stop() }
        // The fake browser never makes a request to the advertised address.
        let browserOpened = Flag()
        let client = service { _ in
            browserOpened.set()
            throw MCPFailure.browser
        }

        await #expect(throws: MCPFailure.discovery) {
            try await client.tools(at: server.endpoint)
        }
        #expect(!browserOpened.isSet)
        #expect(server.registrations.count == 0)
    }

    private func service(
        _ store: MemoryMCPStore = MemoryMCPStore(),
        browser: @escaping MCPBrowser = { _ in throw MCPFailure.browser }
    ) -> MCPService {
        MCPService(store: store, browser: browser)
    }

    private func prepare(_ start: StartingStore, on server: MCPFixture, in credentials: MemoryMCPStore,
                         expiredRegistration: Bool = false) async throws {
        guard start != .empty else { return }
        _ = try await service(credentials, browser: server.openBrowser).tools(at: server.endpoint)
        let login = try #require(credentials.load(server.endpoint))
        let token = try #require(login.token)
        var registration = login.registration
        if expiredRegistration {
            registration = MCPRegistration(
                issuer: registration.issuer, tokenEndpoint: registration.tokenEndpoint,
                redirect: registration.redirect, clientID: registration.clientID, method: registration.method,
                clientSecret: registration.clientSecret, secretExpiry: .distantPast
            )
        }
        try credentials.save(MCPLogin(registration: registration, token: start == .priorGrant ? token : nil),
                             for: server.endpoint)
    }
}

extension MCPTests {
    enum Closing {
        case settings, app, cancel
    }

    @Test(arguments: [Closing.settings, .app, .cancel])
    func closingSettingsOrQuittingClosesAnActiveSignIn(_ closing: Closing) async throws {
        let server = try await MCPFixture.start(protected: true)
        defer { server.stop() }
        let waitingForBrowser = Flag()
        let credentials = MemoryMCPStore()
        let preferences = MemoryPreferences()
        let settings = MCPSettings(service: MCPService(store: credentials, browser: { _ in
            waitingForBrowser.set()
        }), preferences: preferences)
        let view = closing == .cancel
            ? MountedChatContent(VStack { MCPSettingsSection(settings: settings) }, width: 480, height: 180)
            : mount(settings)
        defer {
            settings.cancel()
            view.close()
        }
        settings.address = server.endpoint.absoluteString
        let addition = Task { await settings.addServer() }
        try #require(await eventually { waitingForBrowser.isSet && settings.working })
        #expect(try await server.callbackResponds())
        let saved = try #require(try credentials.snapshot(server.endpoint))

        switch closing {
        case .settings:
            view.close()
        case .app:
            view.layout()
            NotificationCenter.default.post(name: NSApplication.willTerminateNotification, object: NSApp)
        case .cancel:
            try #require(await eventually { view.button(named: "Cancel") != nil })
            let button = try #require(view.button(named: "Cancel"))
            try #require(button.accessibilityPerformPress?() == true)
        }

        try #require(await eventually { !settings.working })
        await addition.value
        try expectCallbackReleased(server)
        #expect(settings.servers.isEmpty)
        #expect(settings.address == server.endpoint.absoluteString)
        #expect(preferences.string(forKey: "mcpServers") == nil)
        #expect(try credentials.snapshot(server.endpoint) == saved)
        #expect(server.registrations.count == 1)
        #expect(server.codeExchanges.count == 0)
    }

    @Test(arguments: MCPFixture.ToolListFailure.allCases)
    func cancelInSettingsEndsAnInvalidToolListWait(_ failure: MCPFixture.ToolListFailure) async throws {
        let server = try await MCPFixture.start(toolListFailure: failure)
        defer { server.stop() }
        let settings = MCPSettings(service: MCPService(store: MemoryMCPStore()), preferences: MemoryPreferences())
        let view = MountedChatContent(VStack { MCPSettingsSection(settings: settings) }, width: 480, height: 180)
        defer {
            settings.cancel()
            view.close()
        }
        settings.address = server.endpoint.absoluteString
        let addition = Task { await settings.addServer() }
        try #require(await eventually { server.invalidToolReplies.count == 1 && settings.working })
        try #require(await eventually { view.button(named: "Cancel") != nil })
        let button = try #require(view.button(named: "Cancel"))
        try #require(button.accessibilityPerformPress?() == true)

        try #require(await eventually { !settings.working })
        await addition.value
        #expect(!settings.canCancel)
        #expect(settings.servers.isEmpty)
        #expect(settings.message == nil)
        #expect(settings.address == server.endpoint.absoluteString)
    }

    private func mount(_ settings: MCPSettings) -> MountedChatContent {
        MountedChatContent(SettingsView(settings: KeySettings(store: InMemoryKeyStore(testKey)),
                                        preferences: ChatPreferences(store: MemoryPreferences()), mcp: settings),
                           width: 480, height: 520)
    }
}

private func expectCallbackReleased(_ server: MCPFixture, sourceLocation: SourceLocation = #_sourceLocation) throws {
    #expect(try !server.callbackAcceptsTCP(), sourceLocation: sourceLocation)
    #expect(try server.callbackPortCanRebind(), sourceLocation: sourceLocation)
}

private func registrationOnlySnapshot(_ server: MCPFixture) throws -> Data {
    try MemoryMCPStore.encode(MCPLogin(registration: server.issuedRegistration()))
}

private final class MemoryMCPStore: MCPCredentialStore {
    let attempts = MCPLoginAttempts()
    private let records = Mutex<[URL: MCPLogin]>([:])
    private struct Writes {
        var count = 0
        var failure: Int?
    }
    private let writes = Mutex(Writes())
    func failWrite(_ number: Int) { writes.withLock { $0.failure = number } }
    func failNextWrite() { writes.withLock { $0.failure = $0.count + 1 } }
    func snapshot(_ server: URL) throws -> Data? {
        try load(server).map(Self.encode)
    }
    static func encode(_ login: MCPLogin) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(login)
    }
    func load(_ server: URL) -> MCPLogin? { records.withLock { $0[server] } }
    func save(_ login: MCPLogin, for server: URL) throws {
        try writes.withLock {
            $0.count += 1
            if $0.count == $0.failure { throw MCPFailure.credentials }
        }
        records.withLock { $0[server] = login }
    }
    func delete(_ server: URL) { records.withLock { $0[server] = nil } }
}
