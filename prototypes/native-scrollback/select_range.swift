// Throwaway #52 typed AX range bridge; no clipboard access.
import AppKit
import ApplicationServices

enum ProbeError: Error {
    case invalidArguments, ambiguousApplication, wrongWindow, wrongElement, wrongSelection
    case accessibility(AXError)
}

func attribute(_ element: AXUIElement, _ name: String) throws -> CFTypeRef {
    var result: CFTypeRef?
    let error = AXUIElementCopyAttributeValue(element, name as CFString, &result)
    guard error == .success else { throw ProbeError.accessibility(error) }
    guard let result else { throw ProbeError.wrongElement }
    return result
}

func element(_ value: CFTypeRef) throws -> AXUIElement {
    guard CFGetTypeID(value) == AXUIElementGetTypeID() else { throw ProbeError.wrongElement }
    return value as! AXUIElement
}

func child(_ parent: AXUIElement, _ role: String) throws -> AXUIElement {
    guard let children = try attribute(parent, kAXChildrenAttribute) as? [AXUIElement] else {
        throw ProbeError.wrongElement
    }
    var matches: [AXUIElement] = []
    for candidate in children {
        if try attribute(candidate, kAXRoleAttribute) as? String == role {
            matches.append(candidate)
        }
    }
    guard matches.count == 1 else { throw ProbeError.wrongElement }
    return matches[0]
}

func run() throws {
    let args = CommandLine.arguments
    guard args.count == 5, let offset = Int(args[1]), let length = Int(args[2]),
          offset >= 0, length > 0, args[3].contains("CLIMON2-52-NATIVE-TEST") else {
        throw ProbeError.invalidArguments
    }
    let apps = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.Terminal")
    guard apps.count == 1 else { throw ProbeError.ambiguousApplication }
    let app = AXUIElementCreateApplication(apps[0].processIdentifier)
    let window = try element(attribute(app, kAXFocusedWindowAttribute))
    guard try attribute(window, kAXTitleAttribute) as? String == args[3],
          try attribute(window, kAXMainAttribute) as? Bool == true else {
        throw ProbeError.wrongWindow
    }
    let group = try child(window, "AXSplitGroup")
    let area = try child(group, kAXScrollAreaRole)
    let text = try child(area, kAXTextAreaRole)
    var range = CFRange(location: offset, length: length)
    guard let value = AXValueCreate(.cfRange, &range) else { throw ProbeError.wrongElement }
    let error = AXUIElementSetAttributeValue(text, kAXSelectedTextRangeAttribute as CFString, value)
    guard error == .success else { throw ProbeError.accessibility(error) }
    guard try attribute(text, kAXSelectedTextAttribute) as? String == args[4] else {
        throw ProbeError.wrongSelection
    }
    print("Known synthetic native selection verified")
}

do {
    try run()
} catch {
    fputs("Native AX range probe failed: \(error)\n", stderr)
    exit(1)
}
