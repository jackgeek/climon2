// Throwaway #52: owned-window metadata and transient image decoding only.
import AppKit
import ApplicationServices

enum CaptureError: Error {
    case arguments, permission, identity, palette, image
}

func rgb(_ color: NSColor) throws -> [Double] {
    guard let value = color.usingColorSpace(.sRGB) else { throw CaptureError.palette }
    return [value.redComponent * 255, value.greenComponent * 255, value.blueComponent * 255]
}

func palette() throws -> [String: Any] {
    guard let settings = CFPreferencesCopyAppValue("Window Settings" as CFString,
                                                  "com.apple.Terminal" as CFString) as? [String: Any],
          let basic = settings["Basic"] as? [String: Any] else { throw CaptureError.palette }
    var result: [String: Any] = ["source": "Terminal Window Settings / Basic; archived NSColor converted to sRGB"]
    for (name, key) in [("blue", "ANSIBlueColor"), ("text", "TextColor"), ("background", "BackgroundColor")] {
        guard let data = basic[key] as? Data,
              let color = try NSKeyedUnarchiver.unarchivedObject(ofClass: NSColor.self, from: data) else {
            throw CaptureError.palette
        }
        result[name] = try rgb(color)
    }
    return result
}

func metadata(_ title: String) throws -> [String: Any] {
    guard CGPreflightScreenCaptureAccess() else { throw CaptureError.permission }
    guard title.contains("CLIMON2-52-NATIVE-TEST") else { throw CaptureError.identity }
    let apps = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.Terminal")
    guard apps.count == 1, apps[0].isActive,
          let windows = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] else {
        throw CaptureError.identity
    }
    let matches = windows.filter {
        ($0[kCGWindowOwnerPID as String] as? Int32) == apps[0].processIdentifier
        && ($0[kCGWindowName as String] as? String)?.contains("CLIMON2-52-NATIVE-TEST") == true
        && ($0[kCGWindowLayer as String] as? Int) == 0
    }
    guard matches.count == 1, matches[0][kCGWindowName as String] as? String == title,
          let id = matches[0][kCGWindowNumber as String] as? Int, id > 0,
          let bounds = matches[0][kCGWindowBounds as String] as? [String: Any],
          let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { throw CaptureError.identity }
    return ["id": id, "owner": "Terminal", "title": title,
            "bounds": [rect.origin.x, rect.origin.y, rect.width, rect.height],
            "permitted": true]
}

func decode(_ input: String, _ output: String) throws -> [String: Any] {
    guard let bitmap = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: input))),
          bitmap.pixelsWide > 0, bitmap.pixelsHigh > 0,
          bitmap.pixelsWide * bitmap.pixelsHigh <= 2_000_000 else { throw CaptureError.image }
    var bytes = Data()
    bytes.reserveCapacity(bitmap.pixelsWide * bitmap.pixelsHigh * 3)
    for y in 0..<bitmap.pixelsHigh {
        for x in 0..<bitmap.pixelsWide {
            guard let color = bitmap.colorAt(x: x, y: y) else { throw CaptureError.image }
            bytes.append(contentsOf: try rgb(color).map { UInt8(max(0, min(255, $0.rounded()))) })
        }
    }
    try bytes.write(to: URL(fileURLWithPath: output), options: .withoutOverwriting)
    return ["dimensions": [bitmap.pixelsWide, bitmap.pixelsHigh], "color_space": "sRGB", "row_order": "top-to-bottom"]
}

func run() throws {
    let args = CommandLine.arguments
    let result: [String: Any]
    if args.count == 3 && args[1] == "--metadata" {
        result = try metadata(args[2])
    } else if args.count == 2 && args[1] == "--palette" {
        result = try palette()
    } else if args.count == 2 && args[1] == "--request-permission" {
        result = ["permitted": CGRequestScreenCaptureAccess()]
    } else if args.count == 4 && args[1] == "--decode" {
        result = try decode(args[2], args[3])
    } else {
        throw CaptureError.arguments
    }
    FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]))
    print("")
}

do {
    try run()
} catch {
    fputs("Window-only pixel capture failed: \(error)\n", stderr)
    exit(1)
}
