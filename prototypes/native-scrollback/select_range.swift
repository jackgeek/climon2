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

func textArea(_ title: String) throws -> AXUIElement {
    guard title.contains("CLIMON2-52-NATIVE-TEST") else {
        throw ProbeError.invalidArguments
    }
    let apps = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.Terminal")
    guard apps.count == 1, apps[0].isActive else { throw ProbeError.ambiguousApplication }
    let app = AXUIElementCreateApplication(apps[0].processIdentifier)
    let window = try element(attribute(app, kAXFocusedWindowAttribute))
    guard try attribute(window, kAXTitleAttribute) as? String == title,
          try attribute(window, kAXMainAttribute) as? Bool == true else {
        throw ProbeError.wrongWindow
    }
    let group = try child(window, "AXSplitGroup")
    let area = try child(group, kAXScrollAreaRole)
    return try child(area, kAXTextAreaRole)
}

func parameter(_ text: AXUIElement, _ name: String, _ range: CFRange) throws -> (CFTypeRef?, [String: Any]) {
    var range = range
    guard let value = AXValueCreate(.cfRange, &range) else { throw ProbeError.wrongElement }
    var result: CFTypeRef?
    let error = AXUIElementCopyParameterizedAttributeValue(text, name as CFString, value, &result)
    if error == .success {
        guard let result else { throw ProbeError.wrongElement }
        return (result, ["status": "observed"])
    }
    let unsupported = error == .attributeUnsupported || error == .parameterizedAttributeUnsupported
        || error == .notImplemented
    return (nil, ["status": unsupported ? "unsupported" : "error", "ax_error": error.rawValue])
}

func bounds(_ text: AXUIElement, _ range: CFRange) throws -> [String: Any] {
    let (value, status) = try parameter(text, kAXBoundsForRangeParameterizedAttribute, range)
    guard let value else { return status }
    guard CFGetTypeID(value) == AXValueGetTypeID() else { throw ProbeError.wrongElement }
    var rect = CGRect.zero
    guard AXValueGetValue(value as! AXValue, .cgRect, &rect) else { throw ProbeError.wrongElement }
    return ["status": "observed", "rect": [rect.origin.x, rect.origin.y, rect.width, rect.height]]
}

func select(_ text: AXUIElement, _ range: CFRange, _ expected: String) throws {
    var range = range
    guard let value = AXValueCreate(.cfRange, &range) else { throw ProbeError.wrongElement }
    let error = AXUIElementSetAttributeValue(text, kAXSelectedTextRangeAttribute as CFString, value)
    guard error == .success else { throw ProbeError.accessibility(error) }
    guard try attribute(text, kAXSelectedTextAttribute) as? String == expected else {
        throw ProbeError.wrongSelection
    }
}

func inspect(_ text: AXUIElement, _ target: String, _ expected: String) throws {
    guard !expected.isEmpty, let native = try attribute(text, kAXValueAttribute) as? String else {
        throw ProbeError.wrongElement
    }
    let range = (native as NSString).range(of: expected, options: .backwards)
    guard range.location != NSNotFound else { throw ProbeError.wrongSelection }
    let selected = CFRange(location: range.location, length: range.length)
    try select(text, selected, expected)
    var names: CFArray?
    let error = AXUIElementCopyParameterizedAttributeNames(text, &names)
    guard error == .success else { throw ProbeError.accessibility(error) }
    guard let names = names as? [String] else { throw ProbeError.wrongElement }
    let visibleValue = try attribute(text, kAXVisibleCharacterRangeAttribute)
    guard CFGetTypeID(visibleValue) == AXValueGetTypeID() else { throw ProbeError.wrongElement }
    var visible = CFRange()
    guard AXValueGetValue(visibleValue as! AXValue, .cfRange, &visible) else {
        throw ProbeError.wrongElement
    }
    let area = try element(attribute(text, kAXParentAttribute))
    let positionValue = try attribute(area, kAXPositionAttribute)
    let sizeValue = try attribute(area, kAXSizeAttribute)
    guard CFGetTypeID(positionValue) == AXValueGetTypeID(),
          CFGetTypeID(sizeValue) == AXValueGetTypeID() else { throw ProbeError.wrongElement }
    var position = CGPoint.zero
    var size = CGSize.zero
    guard AXValueGetValue(positionValue as! AXValue, .cgPoint, &position),
          AXValueGetValue(sizeValue as! AXValue, .cgSize, &size) else {
        throw ProbeError.wrongElement
    }
    let rangeBounds = try bounds(text, selected)
    var units: [[String: Any]] = []
    if rangeBounds["status"] as? String == "observed" {
        for index in 0..<range.length {
            units.append([
                "utf16_index": index,
                "bounds": try bounds(text, CFRange(location: range.location + index, length: 1)),
            ])
        }
    }
    let (styled, styleStatus) = try parameter(text, kAXAttributedStringForRangeParameterizedAttribute, selected)
    var style = styleStatus
    if let styled {
        guard let attributed = styled as? NSAttributedString, attributed.string == expected else {
            throw ProbeError.wrongSelection
        }
        var keys = Set<String>()
        attributed.enumerateAttributes(in: NSRange(location: 0, length: attributed.length)) { attributes, _, _ in
            keys.formUnion(attributes.keys.map { $0.rawValue })
        }
        style["attribute_names"] = keys.sorted()
    }
    let result: [String: Any] = [
        "target": target, "range_verified": true,
        "utf16_length": range.length, "parameterized_attributes": names,
        "range_visible": range.location >= visible.location
            && range.location + range.length <= visible.location + visible.length,
        "viewport_rect": [position.x, position.y, size.width, size.height],
        "bounds": rangeBounds, "unit_bounds": units, "style": style,
    ]
    let data = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
    FileHandle.standardOutput.write(data)
    print("")
}

func run() throws {
    let args = CommandLine.arguments
    guard args.count == 5 else { throw ProbeError.invalidArguments }
    if args[1] == "--observe" {
        try inspect(textArea(args[2]), args[3], args[4])
        return
    }
    guard let offset = Int(args[1]), let length = Int(args[2]), offset >= 0, length > 0 else {
        throw ProbeError.invalidArguments
    }
    let text = try textArea(args[3])
    try select(text, CFRange(location: offset, length: length), args[4])
    print("Known synthetic native selection verified")
}

do {
    try run()
} catch {
    fputs("Native AX range probe failed: \(error)\n", stderr)
    exit(1)
}
