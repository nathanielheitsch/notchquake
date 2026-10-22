// NotchQuake: print candidate notch-panel windows: id, pid, layer, bounds.
// Usage: swift scripts/notchquake.swift   then:  screencapture -x -o -l <id> out.png
import Cocoa

let infos = (CGWindowListCopyWindowInfo(CGWindowListOption.optionAll, 0) as? [[String: AnyObject]]) ?? []
func num(_ k: String, from info: [String: AnyObject]) -> Double {
    if let n = info[k] as? NSNumber { return n.doubleValue }
    return -1
}
for info in infos {
    let layer = Int(num("kCGWindowLayer", from: info))
    guard layer == 27 else { continue }
    let b = (info["kCGWindowBounds"] as? [String: AnyObject]) ?? [:]
    print("id=\(Int(num("kCGWindowNumber", from: info))) pid=\(Int(num("kCGWindowPID", from: info))) layer=\(layer) name=\((info["kCGWindowName"] as? String) ?? "") bounds=(\(num("X", from: b)),\(num("Y", from: b))) \(num("Width", from: b))x\(num("Height", from: b))\n")
}
