import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

GroupBox {
    title: qsTr("声光提醒")

    ColumnLayout {
        anchors.fill: parent
        spacing: 8

        RowLayout {
            Label { text: qsTr("音量") }
            Slider {
                Layout.fillWidth: true
                from: 0
                to: 100
                value: configVm.soundVolume
                onMoved: configVm.soundVolume = value
            }
            Switch {
                text: qsTr("提示音")
                checked: configVm.soundEnabled
                onToggled: configVm.soundEnabled = checked
            }
        }

        RowLayout {
            Label { text: qsTr("亮度") }
            Slider {
                Layout.fillWidth: true
                from: 0
                to: 100
                value: configVm.lightBrightness
                onMoved: configVm.lightBrightness = value
            }
            Switch {
                text: qsTr("提示灯")
                checked: configVm.lightEnabled
                onToggled: configVm.lightEnabled = checked
            }
        }

        RowLayout {
            Label { text: qsTr("时长 ms") }
            SpinBox {
                from: 0
                to: 60000
                stepSize: 500
                value: configVm.lightDurationMs
                onValueModified: configVm.lightDurationMs = value
            }
            Label { text: qsTr("R") }
            SpinBox {
                from: 0
                to: 255
                value: configVm.lightR
                onValueModified: configVm.lightR = value
            }
            Label { text: qsTr("G") }
            SpinBox {
                from: 0
                to: 255
                value: configVm.lightG
                onValueModified: configVm.lightG = value
            }
            Label { text: qsTr("B") }
            SpinBox {
                from: 0
                to: 255
                value: configVm.lightB
                onValueModified: configVm.lightB = value
            }
        }

        RowLayout {
            Button {
                text: qsTr("预览")
                onClicked: configVm.applySoundLight(true)
            }
            Button {
                text: qsTr("保存到设备")
                onClicked: configVm.applySoundLight(false)
            }
        }
    }
}
