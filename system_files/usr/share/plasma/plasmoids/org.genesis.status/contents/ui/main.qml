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
    property bool lastWasApp: false
    property bool sandbox: false
    property bool voice: false
    property var netDomains: []
    property int jobs: 0
    property int running: 0
    property string runningWhat: ""
    property string runningId: ""
    property string staged: ""

    Plasmoid.icon: "genesis"
    Plasmoid.status: PlasmaCore.Types.ActiveStatus
    toolTipMainText: alive ? "Genesis · on this machine" : "Genesis · starting"
    toolTipSubText: alive ? ((running > 0 ? running + " running · " : "") + "Model " + model + (netDomains.length ? " · network used: " + netDomains.join(", ") : " · no network used") + (staged ? " · update ready, restart to apply" : "")) : "The local agent service is not running yet"

    // Reads go over this user's Unix socket, with the token: the loopback port answers only its own user,
    // and session lists are no longer open to a bare GET. Each read is its own source (the engine merges
    // identical ones), and its callback waits in `pending` until the output arrives.
    property var pending: ({})
    property int seq: 0
    readonly property string curl: "curl -fsS -m 5 --unix-socket \"$XDG_RUNTIME_DIR/genesis/agentd.sock\" -H \"X-Genesis-Token: $(cat \"$XDG_RUNTIME_DIR/genesis/agentd.token\")\" "
    P5Support.DataSource {
        id: reader; engine: "executable"
        onNewData: function(source, data) {
            var cb = root.pending[source]; delete root.pending[source]; disconnectSource(source)
            var j = null; try { j = JSON.parse(data["stdout"]) } catch (e) {}
            if (cb) cb(j)
        }
    }
    function get(path, cb) {
        var src = root.curl + "http://localhost" + path + " # " + (++root.seq)
        root.pending[src] = cb
        reader.connectSource(src)
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
            var apps = (last && last.apps) ? last.apps : []
            lastJob = last ? (apps.length ? ("installed " + apps.join(", ")) : ((last.asked || "a job") + " · " + (last.changes || 0) + " changes")) : ""
            lastWasApp = apps.length > 0
            lastJobId = last ? last.id : ""
        })
        get("/api/sessions", function(list) {
            if (!list) return
            var d = {}; var n = 0; var act = 0; var what = ""; var id = ""
            for (var i = 0; i < list.length; i++) {
                n++
                var st = list[i].state || ""
                if (st === "running" || st === "waiting") { act++; if (id === "") { id = list[i].id; what = (list[i].project || "").split("/").pop() + (st === "waiting" ? " · waiting for you" : "") } }
                var nu = list[i].network_uses || []; for (var j = 0; j < nu.length; j++) d[nu[j]] = true
            }
            jobs = n; running = act; runningWhat = what; runningId = id
            netDomains = Object.keys(d).map(function(k) { return k === "*" ? "a command" : k })
        })
    }
    Timer { interval: 20000; running: true; repeat: true; triggeredOnStart: true; onTriggered: root.refresh() }
    // Meta+Shift+M does the same from the keyboard; both go through genesis-mode
    P5Support.DataSource { id: modeRunner; engine: "executable"; onNewData: function(source, data) { disconnectSource(source); root.refresh() } }

    // Undo from the panel: the same call the maker's toast makes, with the token from the runtime directory.
    // Stop what is running, from the panel, without opening anything
    function stopRunning() {
        if (runningId === "") return
        modeRunner.connectSource(root.curl + "-X POST http://localhost/api/sessions/" + runningId + "/stop")
    }

    function undoLast() {
        if (lastJobId === "") return
        modeRunner.connectSource(root.curl + "-X POST http://localhost/api/history/" + lastJobId + "/undo")
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
        RowLayout { visible: root.alive && root.running > 0; Layout.fillWidth: true
            PC.Label { text: root.running + (root.running === 1 ? " job running: " : " jobs running: ") + root.runningWhat; Layout.fillWidth: true; wrapMode: Text.Wrap }
            PC.Button { text: "Stop"; icon.name: "process-stop"; onClicked: root.stopRunning() }
        }
        PC.Label { visible: root.alive && root.running === 0; text: "Nothing running."; opacity: 0.8; Layout.fillWidth: true }
        PC.Label { visible: root.alive && root.lastJob !== ""; text: (root.lastWasApp ? "" : "Last change: ") + root.lastJob + (root.lastWasApp ? " · its settings stay behind" : ""); Layout.fillWidth: true; wrapMode: Text.Wrap; opacity: 0.9 }
        RowLayout { visible: root.alive && root.lastJobId !== ""; Layout.fillWidth: true
            PC.Button { text: root.lastWasApp ? "Remove it again" : "Undo the last job"; icon.name: "edit-undo"; onClicked: { root.undoLast(); root.expanded = false } }
            PC.Button { text: "See what changed"; icon.name: "document-edit"; onClicked: { modeRunner.connectSource("genesis-window http://127.0.0.1:11520/settings#sec-undo"); root.expanded = false } }
        }
        PC.Button { text: "Open Genesis"; icon.name: "genesis"; onClicked: { root.openMaker(); root.expanded = false } }
    }
}
