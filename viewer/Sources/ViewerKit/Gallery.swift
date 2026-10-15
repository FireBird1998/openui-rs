import Foundation

/// A program that uses every component the viewer draws natively, against
/// upstream's example library (the corpus's `bench` schema). It parses with
/// no errors; File > Open Component Gallery opens it.
public enum Gallery {
    static let library = "bench"

    static let program = #"""
root = Stack([header, kpis, charts, stories, tip, tabs, notes], "column", "l")
header = CardHeader("Component gallery", "Every component the viewer draws natively, from upstream's example library")
kpis = Stack([readers, published, corrections], "row", "m")
readers = Card([TextContent("Readers today", "small"), TextContent("12,480", "large-heavy"), Tag("+8% on yesterday", null, "sm", "success")])
published = Card([TextContent("Stories published", "small"), TextContent("342", "large-heavy")], "sunk")
corrections = Card([TextContent("Corrections", "small"), TextContent("3", "large-heavy")], "clear")
charts = Stack([perDay, byDesk, sources], "row", "m")
perDay = Card([CardHeader("Readers per day"), LineChart(["Mon", "Tue", "Wed", "Thu", "Fri"], [Series("Web", [120, 180, 150, 210, 260]), Series("App", [80, 95, 140, 160, 190])], "natural")])
byDesk = Card([CardHeader("Stories by desk"), BarChart(["World", "Tech", "Sport"], [Series("Published", [42, 31, 27]), Series("Drafts", [8, 12, 5])], "grouped")])
sources = Card([CardHeader("Where readers come from"), PieChart([Slice("Search", 45), Slice("Social", 30), Slice("Direct", 25)], "donut")])
stories = Card([CardHeader("Latest stories", "A table with a number column and an action column"), Table([Col("Headline"), Col("Reads", "number"), Col("Open", "action")], [["Rates hold steady", 5120, "Open"], ["New chip unveiled", 3870, "Open"], ["Derby ends in a draw", 2210, "Open"]])])
tip = Card([CardHeader("Send a tip"), Form("tip", [FormControl("Your email", Input("email", "you@example.com", "email")), FormControl("Desk", Select("desk", [SelectItem("world", "World"), SelectItem("tech", "Tech"), SelectItem("sport", "Sport")], "Pick a desk"), "Where your tip should go")], Buttons([Button("Send", "send", "primary"), Button("Save draft", "save", "secondary"), Button("Discard", "discard", "tertiary", "destructive")]))])
tabs = Card([Tabs([TabItem("summary", "Summary", [TextContent("Tabs show one item's content at a time; the viewer lets you switch.")]), TabItem("tags", "Tags", [Stack([Tag("Neutral"), Tag("Info", null, "md", "info"), Tag("Success", null, "md", "success"), Tag("Warning", null, "md", "warning"), Tag("Danger", null, "md", "danger")], "row", "s", "start", "start", true)])])])
notes = Stack([Callout("info", "Heads up", "Callouts come in five variants."), Callout("success", "Saved", "Your changes are live."), Callout("warning", "Slow feed", "One source is lagging."), Callout("error", "Failed", "Couldn't reach the server."), TextCallout("neutral", "A text callout", "Drawn the same way.")], "column", "s")
"""#

    static var request: ProgramRequest {
        ProgramRequest(title: "Component gallery", text: program, library: .corpus(library), root: nil)
    }

    static let openNotification = Notification.Name("dev.openui-rs.viewer.openGallery")

    /// Asks the window to open the gallery.
    @MainActor public static func open() {
        NotificationCenter.default.post(name: openNotification, object: nil)
    }
}
