import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    id: root
    property var sys: null
    property string status: ""
    property bool reachable: false

    function api(method, path, body, cb) {
        var x = new XMLHttpRequest()
        x.onreadystatechange = function() { if (x.readyState === XMLHttpRequest.DONE) { var j = null; try { j = JSON.parse(x.responseText) } catch (e) {}; cb(x.status, j) } }
        x.open(method, "http://127.0.0.1:11520" + path)
        if (body) { x.setRequestHeader("Content-Type", "application/json"); x.send(JSON.stringify(body)) } else { x.send() }
    }
    function refresh() { api("GET", "/api/system", null, function(st, j) { reachable = (st === 200 && j); if (reachable) sys = j }) }
    Component.onCompleted: refresh()
    Timer { interval: 5000; running: true; repeat: true; onTriggered: root.refresh() }

    Kirigami.FormLayout {
        Kirigami.InlineMessage {
            Kirigami.FormData.isSection: true
            Layout.fillWidth: true
            visible: !root.reachable
            type: Kirigami.MessageType.Warning
            text: "The Genesis service is not running for this session yet. Log in to the desktop and reopen this page."
        }

        Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "Updates" }
        QQC2.Label { Kirigami.FormData.label: "Running:"; text: sys && sys.os ? (sys.os.pretty || sys.os.name) + (sys.os.built ? "  (built " + sys.os.built.substring(0, 10) + ")" : "") : "…" }
        QQC2.Label { Kirigami.FormData.label: "Image:"; text: sys && sys.os && sys.os.image ? sys.os.image : "not a bootc install"; font.family: "monospace" }
        QQC2.Label { Kirigami.FormData.label: "Staged update:"; text: sys && sys.os ? (sys.os.staged ? "ready, built " + (sys.os.staged_built || "").substring(0, 10) + ". Restart to apply." : (sys.os.unit_active ? "checking…" : "none")) : "…" }
        QQC2.Button { text: "Check for updates now"; icon.name: "view-refresh"; enabled: root.reachable; onClicked: { status = "checking in the background…"; root.api("POST", "/api/system/update", {}, function(st, j) { status = (st === 202) ? "Checking. A staged update shows here when ready." : ((j && j.error) || "Could not start the check.") }) } }
        QQC2.Label { text: root.status; visible: root.status !== ""; opacity: 0.7 }

        Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "On this machine" }
        QQC2.Label { Kirigami.FormData.label: "Models run on:"; text: { if (!sys || !sys.profile) return "…"; var g = sys.profile.gpus && sys.profile.gpus[0]; return (g ? (g.name || "GPU") : "the CPU") + (sys.profile.ram_mb ? " · " + Math.round(sys.profile.ram_mb / 1024) + " GB RAM" : "") } }
        QQC2.Label { Kirigami.FormData.label: "Model service:"; text: sys && sys.router && sys.router.running ? "running" : "not reachable" }
        QQC2.Label { Kirigami.FormData.label: "Speed:"; text: sys && sys.last_generation ? sys.last_generation.tokens_per_second + " tokens/s (last reply, " + sys.last_generation.model + ")" : "measured on the first reply" }
        QQC2.Label { Kirigami.FormData.label: "Cloud:"; text: sys && sys.setup && sys.setup.cloud_enabled ? "allowed by you at first run" : "off, nothing leaves this computer" }

        Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "Models on disk" }
        Repeater {
            model: sys && sys.models_on_disk ? sys.models_on_disk : []
            QQC2.Label { Kirigami.FormData.label: modelData.role + ":"; text: modelData.file + "  " + modelData.size_gb + " GB"; font.family: "monospace" }
        }
        QQC2.Label { visible: sys && (!sys.models_on_disk || sys.models_on_disk.length === 0); text: "No models downloaded yet. Open the Genesis first-run wizard from the app menu."; opacity: 0.7 }

        Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "How much Genesis may do on its own" }
        QQC2.ComboBox {
            id: mode
            Kirigami.FormData.label: "Default for new jobs:"
            textRole: "text"; valueRole: "value"
            model: [
                { text: "Ask: asks before every change", value: "assist" },
                { text: "Trusted: works inside the project, asks for installs, network and anything else", value: "auto_edit" },
                { text: "Hands-off: also outside the project, everything snapshotted; system changes still ask", value: "autonomous" }
            ]
            Component.onCompleted: { var m = sys && sys.user && sys.user.default_mode; if (m) currentIndex = indexOfValue(m) }
            onActivated: root.api("POST", "/api/system/mode", { mode: currentValue }, function(st, j) { status = st === 200 ? "Saved." : "Could not save." })
        }

        Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "Where requests go" }
        QQC2.Label { Kirigami.FormData.label: "Now:"; text: sys && sys.provider ? (sys.provider.cloud ? "Claude (cloud, " + sys.provider.model + ")" : "on this machine (local models)") : "…" }
        QQC2.RadioButton { id: provLocal; text: "On this machine (local models)"; checked: !(sys && sys.provider && sys.provider.cloud); onClicked: root.api("POST", "/api/system/provider", { provider: "local" }, function() { root.refresh() }) }
        QQC2.RadioButton { id: provClaude; text: "Claude (cloud, with your Anthropic API key)"; checked: sys && sys.provider && sys.provider.cloud; onClicked: { if (sys && sys.provider && sys.provider.key_set) root.api("POST", "/api/system/provider", { provider: "claude" }, function() { root.refresh() }); else status = "Enter an API key below, then Save." } }
        RowLayout { Kirigami.FormData.label: "API key:"; QQC2.TextField { id: keyField; echoMode: TextInput.Password; placeholderText: sys && sys.provider && sys.provider.key_set ? "a key is stored" : "sk-ant-…"; Layout.preferredWidth: 320 }
            QQC2.Button { text: "Save"; enabled: keyField.text.trim() !== ""; onClicked: root.api("POST", "/api/system/provider", { provider: "claude", claude_api_key: keyField.text.trim() }, function(st, j) { keyField.text = ""; status = (j && j.ok) ? "Saved. Requests now go to Claude." : "Could not save."; root.refresh() }) }
            QQC2.Button { text: "Forget"; visible: sys && sys.provider && sys.provider.key_set; onClicked: root.api("POST", "/api/system/provider", { provider: "local", clear_key: true }, function() { status = "Key removed. Requests stay local."; root.refresh() }) } }
        QQC2.Label { text: "With Claude on, every request from the maker, the palette and ask goes to Anthropic. Genesis shows it everywhere."; opacity: 0.7; wrapMode: Text.Wrap; Layout.fillWidth: true }

        Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "Look" }
        RowLayout {
            Kirigami.FormData.label: "Day / night:"
            QQC2.Button { text: "Genesis Dark"; onClicked: root.api("POST", "/api/system/theme", { theme: "dark" }, function() {}) }
            QQC2.Button { text: "Genesis Light"; onClicked: root.api("POST", "/api/system/theme", { theme: "light" }, function() {}) }
            QQC2.Label { text: "also Meta+Shift+T"; opacity: 0.6 }
        }

        Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "Voice" }
        QQC2.Label { Kirigami.FormData.label: "Listening:"; text: sys && sys.voice && sys.voice.input ? "ready (whisper.cpp on this machine)" : "no speech model yet (optional in every pack)" }
        QQC2.Label { Kirigami.FormData.label: "Speaking:"; text: sys && sys.voice && sys.voice.output ? "ready (Piper on this machine)" : "no voice yet (optional in every pack)" }

        Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "Undo history" }
        QQC2.Label { text: sys && sys.history && sys.history.length ? sys.history.length + " job(s) changed something outside their project; each was snapshotted first. Undo from the maker, or with genesis-txd undo <id>." : "No job has changed anything outside its project yet."; wrapMode: Text.Wrap; Layout.fillWidth: true }
    }
}
