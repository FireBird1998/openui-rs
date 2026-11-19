import Foundation
import OpenUINative

extension ProgramRequest {
    static func responseSample(bundle: Bool) throws -> Self {
        let file = bundle ? "edition-bundle" : "edition-message"
        guard let url = Bundle.module.url(forResource: file, withExtension: "oui", subdirectory: "Samples") else {
            throw DecodeError(description: "Missing sample \(file)")
        }
        let script = bundle ? try ToolScript(json: #"{"edition":[{"value":"Evening edition"}]}"#) : nil
        return try Self(title: bundle ? "Edition bundle" : "Edition message", text: String(contentsOf: url, encoding: .utf8),
                        library: .corpus("bench"), toolScript: script)
    }
}
