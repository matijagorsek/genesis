// Genesis status: proves "on this machine". Polls the local agent daemon (no network involved) and
// shows the model, whether the sandbox is on, and whether any job used the network.
import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.components as PC
import org.kde.kirigami as Kirigami
import org.kde.plasma.plasma5support as P5Support

PlasmoidItem {
    id: root
    property bool alive: false
    property string model: ""
    property string mode: ""
    property string lastJob: ""
    property string lastJobId: ""
    property bool sandbox: false
    property bool voice: false
    property var netDomains: []
    property int jobs: 0
    property string staged: ""

    Plasmoid.icon: "genesis"
    Plasmoid.status: PlasmaCore.Types.ActiveStatus
    toolTipMainText: alive ? "Genesis · on this machine" : "Genesis · starting"
    toolTipSubText: alive ? ("Model " + model + (netDomains.length ? " · network used: " + netDomains.join(", ") : " · no network used") + (staged ? " · update ready, restart to apply" : "")) : "The local agent service is not running yet"

    function get(path, cb) {
        var x = new XMLHttpRequest()
        x.onreadystatechange = function() { if (x.readyState === XMLHttpRequest.DONE) { try { cb(x.status === 200 ? JSON.parse(x.responseText) : null) } catch (e) { cb(null) } } }
        x.open("GET", "http://127.0.0.1:11520" + path); x.send()
    }
    function refresh() {
        get("/api/health", function(h) {
            alive = !!(h && h.ok); if (!h) return
            model = h.model || ""; sandbox = !!h.sandbox; voice = !!h.voice; mode = h.default_mode || ""
        })
        get("/api/system", function(sy) { staged = (sy && sy.os && sy.os.staged) ? (sy.os.staged_built || "").substring(0, 10) : "" })
        // the last job that changed something outside its project, so undo is one click away from the panel
        get("/api/system", function(sy) {
            var h = (sy && sy.history) ? sy.history : []
            var last = null
            for (var i = 0; i < h.length; i++) { if (/committed|kept|done/i.test(h[i].status || "")) { last = h[i]; break } }
            lastJob = last ? ((last.asked || "a job") + " · " + (last.changes || 0) + " changes") : ""
            lastJobId = last ? last.id : ""
        })
        get("/api/sessions", function(list) {
            if (!list) return
            var d = {}; var n = 0
            for (var i = 0; i < list.length; i++) { n++; var nu = list[i].network_uses || []; for (var j = 0; j < nu.length; j++) d[nu[j]] = true }
            jobs = n; netDomains = Object.keys(d).map(function(k) { return k === "*" ? "a command" : k })
        })
    }
    Timer { interval: 20000; running: true; repeat: true; triggeredOnStart: true; onTriggered: root.refresh() }
    // Meta+Shift+M does the same from the keyboard; both go through genesis-mode
    P5Support.DataSource { id: modeRunner; engine: "executable"; onNewData: function(source, data) { disconnectSource(source); root.refresh() } }

    // Undo from the panel: the same call the maker's toast makes, with the token from the runtime directory.
    function undoLast() {
        if (lastJobId === "") return
        modeRunner.connectSource("sh -lc 'curl -s -X POST -H \"X-Genesis-Token: $(cat $XDG_RUNTIME_DIR/genesis/agentd.token)\" http://127.0.0.1:11520/api/history/" + lastJobId + "/undo'")
    }

    P5Support.DataSource { id: exec; engine: "executable"; onNewData: function(source) { disconnectSource(source) } }
    function openMaker() { exec.connectSource("genesis-window http://127.0.0.1:11520/") }

    compactRepresentation: MouseArea {
        Accessible.name: root.toolTipMainText + ". " + root.toolTipSubText
        Accessible.role: Accessible.Button
        Layout.minimumWidth: Kirigami.Units.iconSizes.small
        Layout.minimumHeight: Kirigami.Units.iconSizes.small
        onClicked: root.expanded = !root.expanded
        Kirigami.Icon { anchors.fill: parent; source: "genesis"; opacity: root.alive ? 1 : 0.5 }
        Rectangle { visible: root.netDomains.length > 0 || root.staged !== ""; width: 7; height: 7; radius: 3.5; color: root.staged !== "" ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.neutralTextColor; anchors.right: parent.right; anchors.bottom: parent.bottom }
    }

    fullRepresentation: ColumnLayout {
        Layout.preferredWidth: Kirigami.Units.gridUnit * 18
        spacing: Kirigami.Units.smallSpacing
        RowLayout { Kirigami.Icon { source: "genesis"; Layout.preferredWidth: Kirigami.Units.iconSizes.medium; Layout.preferredHeight: Kirigami.Units.iconSizes.medium }
            PC.Label { text: root.alive ? "On this machine" : "Genesis is starting"; font.bold: true; Layout.fillWidth: true } }
        PC.Label { text: root.alive ? "Model: " + root.model : "The local agent service is not up yet."; Layout.fillWidth: true; wrapMode: Text.Wrap }
        RowLayout { visible: root.alive; Layout.fillWidth: true
            PC.Label { text: "Mode: " + (root.mode === "assist" ? "Ask" : root.mode === "autonomous" ? "Hands-off" : "Trusted"); Layout.fillWidth: true }
            PC.Button { text: "Cycle"; icon.name: "view-refresh"; onClicked: modeRunner.connectSource("genesis-mode cycle") }
        }
        PC.Label { visible: root.alive; text: root.netDomains.length ? "Network used by a job: " + root.netDomains.join(", ") : "No job used the network."; color: root.netDomains.length ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.positiveTextColor; Layout.fillWidth: true; wrapMode: Text.Wrap }
        PC.Label { visible: root.alive; text: (root.sandbox ? "Commands run in a sandbox. " : "") + (root.voice ? "Voice input ready." : ""); Layout.fillWidth: true; wrapMode: Text.Wrap; opacity: 0.8 }
        PC.Label { visible: root.staged !== ""; text: "A Genesis update built " + root.staged + " is ready. It applies when you restart."; color: Kirigami.Theme.positiveTextColor; Layout.fillWidth: true; wrapMode: Text.Wrap }
        PC.Label { text: "Nothing you type or say leaves this computer."; opacity: 0.7; Layout.fillWidth: true; wrapMode: Text.Wrap }
        PC.Label { visible: root.alive && root.lastJob !== ""; text: "Last change: " + root.lastJob; Layout.fillWidth: true; wrapMode: Text.Wrap; opacity: 0.9 }
        RowLayout { visible: root.alive && root.lastJobId !== ""; Layout.fillWidth: true
            PC.Button { text: "Undo the last job"; icon.name: "edit-undo"; onClicked: { root.undoLast(); root.expanded = false } }
            PC.Button { text: "See what changed"; icon.name: "document-edit"; onClicked: { modeRunner.connectSource("genesis-window http://127.0.0.1:11520/settings#sec-undo"); root.expanded = false } }
        }
        PC.Button { text: "Open Genesis"; icon.name: "genesis"; onClicked: { root.openMaker(); root.expanded = false } }
    }
}
