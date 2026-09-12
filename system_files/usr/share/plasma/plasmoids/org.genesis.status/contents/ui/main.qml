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
    property bool sandbox: false
    property bool voice: false
    property var netDomains: []
    property int jobs: 0

    Plasmoid.icon: "genesis"
    Plasmoid.status: PlasmaCore.Types.ActiveStatus
    toolTipMainText: alive ? "Genesis · on this machine" : "Genesis · starting"
    toolTipSubText: alive ? ("Model " + model + (netDomains.length ? " · network used: " + netDomains.join(", ") : " · no network used")) : "The local agent service is not running yet"

    function get(path, cb) {
        var x = new XMLHttpRequest()
        x.onreadystatechange = function() { if (x.readyState === XMLHttpRequest.DONE) { try { cb(x.status === 200 ? JSON.parse(x.responseText) : null) } catch (e) { cb(null) } } }
        x.open("GET", "http://127.0.0.1:11520" + path); x.send()
    }
    function refresh() {
        get("/api/health", function(h) {
            alive = !!(h && h.ok); if (!h) return
            model = h.model || ""; sandbox = !!h.sandbox; voice = !!h.voice
        })
        get("/api/sessions", function(list) {
            if (!list) return
            var d = {}; var n = 0
            for (var i = 0; i < list.length; i++) { n++; var nu = list[i].network_uses || []; for (var j = 0; j < nu.length; j++) d[nu[j]] = true }
            jobs = n; netDomains = Object.keys(d).map(function(k) { return k === "*" ? "a command" : k })
        })
    }
    Timer { interval: 5000; running: true; repeat: true; triggeredOnStart: true; onTriggered: root.refresh() }

    P5Support.DataSource { id: exec; engine: "executable"; onNewData: function(source) { disconnectSource(source) } }
    function openMaker() { exec.connectSource("genesis-window http://127.0.0.1:11520/") }

    compactRepresentation: MouseArea {
        Layout.minimumWidth: Kirigami.Units.iconSizes.small
        Layout.minimumHeight: Kirigami.Units.iconSizes.small
        onClicked: root.expanded = !root.expanded
        Kirigami.Icon { anchors.fill: parent; source: "genesis"; opacity: root.alive ? 1 : 0.5 }
        Rectangle { visible: root.netDomains.length > 0; width: 7; height: 7; radius: 3.5; color: Kirigami.Theme.neutralTextColor; anchors.right: parent.right; anchors.bottom: parent.bottom }
    }

    fullRepresentation: ColumnLayout {
        Layout.preferredWidth: Kirigami.Units.gridUnit * 18
        spacing: Kirigami.Units.smallSpacing
        RowLayout { Kirigami.Icon { source: "genesis"; Layout.preferredWidth: Kirigami.Units.iconSizes.medium; Layout.preferredHeight: Kirigami.Units.iconSizes.medium }
            PC.Label { text: root.alive ? "On this machine" : "Genesis is starting"; font.bold: true; Layout.fillWidth: true } }
        PC.Label { text: root.alive ? "Model: " + root.model : "The local agent service is not up yet."; Layout.fillWidth: true; wrapMode: Text.Wrap }
        PC.Label { visible: root.alive; text: root.netDomains.length ? "Network used by a job: " + root.netDomains.join(", ") : "No job used the network."; color: root.netDomains.length ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.positiveTextColor; Layout.fillWidth: true; wrapMode: Text.Wrap }
        PC.Label { visible: root.alive; text: (root.sandbox ? "Commands run in a sandbox. " : "") + (root.voice ? "Voice input ready." : ""); Layout.fillWidth: true; wrapMode: Text.Wrap; opacity: 0.8 }
        PC.Label { text: "Nothing you type or say leaves this computer."; opacity: 0.7; Layout.fillWidth: true; wrapMode: Text.Wrap }
        PC.Button { text: "Open Genesis"; icon.name: "genesis"; onClicked: { root.openMaker(); root.expanded = false } }
    }
}
