// Genesis splash: the wallpaper, the mark, a thin progress line. Same composition as boot and login.
import QtQuick

Rectangle {
    id: root
    color: "#0b0f14"
    property int stage
    onStageChanged: if (stage === 1) introAnimation.running = true

    Image {
        anchors.fill: parent
        source: "file:///usr/share/wallpapers/Genesis/contents/images/2560x1600.png"
        fillMode: Image.PreserveAspectCrop
        opacity: 0.9
    }
    Item {
        id: content
        anchors.centerIn: parent
        width: 220; height: 160
        opacity: 0
        Image { id: logo; source: "images/genesis.svg"; width: 96; height: 96; sourceSize: Qt.size(96, 96); anchors.horizontalCenter: parent.horizontalCenter }
        Text { anchors.top: logo.bottom; anchors.topMargin: 18; anchors.horizontalCenter: parent.horizontalCenter; text: "Genesis"; color: "#dfe6ee"; font.pixelSize: 22; font.weight: Font.DemiBold; font.letterSpacing: 1 }
    }
    Rectangle {
        anchors.bottom: parent.bottom; anchors.bottomMargin: 60; anchors.horizontalCenter: parent.horizontalCenter
        width: 220; height: 2; radius: 1; color: "#232c37"
        Rectangle { width: parent.width * (root.stage / 6); height: parent.height; radius: 1; color: "#5fb5bd"; Behavior on width { NumberAnimation { duration: 250 } } }
    }
    OpacityAnimator { id: introAnimation; target: content; from: 0; to: 1; duration: 700; running: false }
}
